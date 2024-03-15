use chrono::Local;

pub enum LogServiceType {
    Register,
    Database,
    Scoket,
    Source,
    Other
}
impl LogServiceType {
    fn as_str(&self) -> &'static str {
        match self {
            LogServiceType::Register => "REGISTER",
            LogServiceType::Database => "DATABASE",
            LogServiceType::Scoket => "SOCKET",
            LogServiceType::Source => "SOURCE",
            LogServiceType::Other => "OTHER"
        }
    }
}

pub fn log_info(service: LogServiceType, message: String) {
    println!("{} - {} - {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), service.as_str(), message)
}

pub fn log_error(service: LogServiceType, message: String) {
    println!("{} - ERROR - {} - {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), service.as_str(), message)
}