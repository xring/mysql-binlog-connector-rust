use crate::{
    binlog_error::BinlogError,
    constants::MysqlRespCode,
    event::checksum_type::ChecksumType,
    network::{
        error_packet::ErrorPacket, greeting_packet::GreetingPacket, packet_channel::PacketChannel,
        result_set_row_packet::ResultSetRowPacket,
    },
};

use super::{
    authenticate_command::AuthenticateCommand, dump_binlog_command::DumpBinlogCommand,
    query_command::QueryCommand,
};

pub struct CommandUtil {}

impl CommandUtil {
    pub fn connect_and_authenticate(
        hostname: String,
        port: String,
        username: String,
        password: String,
        schema: String,
    ) -> Result<PacketChannel, BinlogError> {
        // connect to hostname:port
        let mut channel = PacketChannel::new(hostname.clone(), port.clone())?;

        // read and parse greeting packet
        let (greeting_buf, sequence) = channel.read_with_sequece()?;
        let greeting_packet = GreetingPacket::new(greeting_buf)?;

        let mut command = AuthenticateCommand {
            schema: schema.clone(),
            username: username.clone(),
            password: password.clone(),
            scramble: greeting_packet.scramble.clone(),
            collation: greeting_packet.server_collation,
        };
        // send authenticate command
        channel.write(&command.to_bytes()?, sequence + 1)?;
        // check result
        let res_buf = channel.read()?;
        Self::parse_result(&res_buf)?;

        Ok(channel)
    }

    pub fn execute_query(
        channel: &mut PacketChannel,
        sql: String,
    ) -> Result<Vec<ResultSetRowPacket>, BinlogError> {
        Self::execute_sql(channel, sql)?;
        // read to EOF
        while channel.read()?[0] != MysqlRespCode::EOF {}
        // get result sets
        let mut result_sets = Vec::new();

        let mut buf = channel.read()?;
        while buf[0] != MysqlRespCode::EOF {
            Self::check_error_packet(&buf)?;
            let result_set = ResultSetRowPacket::new(&buf)?;
            result_sets.push(result_set);
            buf = channel.read()?;
        }

        Ok(result_sets)
    }

    pub fn execute_sql(channel: &mut PacketChannel, sql: String) -> Result<(), BinlogError> {
        let mut command = QueryCommand { sql };

        // send the query command, sequence for non-authenticate commands are always 0
        channel.write(&command.to_bytes()?, 0)?;

        // read the response packet
        let buf = channel.read()?;
        Self::check_error_packet(&buf)
    }

    pub fn fetch_binlog_info(channel: &mut PacketChannel) -> Result<(String, u64), BinlogError> {
        let result_sets = CommandUtil::execute_query(channel, "show master status".to_string())?;
        if result_sets.len() == 0 {
            return Err(BinlogError::MysqlError {
                error: "failed to fetch binlog filename and position".to_string(),
            });
        }
        let binlog_filename = result_sets[0].values[0].clone();
        let binlog_position = result_sets[0].values[1].clone().parse::<u64>()?;
        Ok((binlog_filename, binlog_position))
    }

    pub fn fetch_binlog_checksum(channel: &mut PacketChannel) -> Result<ChecksumType, BinlogError> {
        let result_set_rows =
            CommandUtil::execute_query(channel, "select @@global.binlog_checksum".to_string())?;
        let mut checksum_name = "";
        if result_set_rows.len() > 0 {
            checksum_name = result_set_rows[0].values[0].as_str();
        }
        Ok(ChecksumType::from_name(checksum_name))
    }

    pub fn setup_binlog_connection(channel: &mut PacketChannel) -> Result<(), BinlogError> {
        let mut command = QueryCommand {
            sql: "set @master_binlog_checksum= @@global.binlog_checksum".to_string(),
        };
        channel.write(&command.to_bytes()?, 0)?;

        let buf = channel.read()?;
        CommandUtil::check_error_packet(&buf)?;
        Ok(())
    }

    pub fn dump_binlog(
        channel: &mut PacketChannel,
        binlog_filename: String,
        binlog_position: u64,
        server_id: u64,
    ) -> Result<(), BinlogError> {
        let mut command = DumpBinlogCommand {
            binlog_filename,
            binlog_position,
            server_id,
        };

        let buf = command.to_bytes()?;
        channel.write(&buf, 0)?;
        Ok(())
    }

    fn parse_result(buf: &Vec<u8>) -> Result<(), BinlogError> {
        match buf[0] {
            MysqlRespCode::OK => Ok(()),

            MysqlRespCode::ERROR => Self::check_error_packet(&buf),

            // TODO
            MysqlRespCode::AUTH_PLUGIN_SWITCH => Err(BinlogError::MysqlError {
                error: "unsupported mysql response code: AUTH_PLUGIN_SWITCH".to_string(),
            }),

            _ => Err(BinlogError::MysqlError {
                error: "connect mysql failed".to_string(),
            }),
        }
    }

    fn check_error_packet(buf: &Vec<u8>) -> Result<(), BinlogError> {
        if buf[0] == MysqlRespCode::ERROR {
            let _error_packet = ErrorPacket::new(buf)?;
            return Err(BinlogError::MysqlError {
                error: "connect mysql failed, mysql response code: ERROR".to_string(),
            });
        }
        Ok(())
    }
}