use chrono::Local;

pub enum LogServiceType {
    Register,
    Database,
    Other
}
impl LogServiceType {
    fn as_str(&self) -> &'static str {
        match self {
            LogServiceType::Register => "REGISTER",
            LogServiceType::Database => "DATABASE",
            LogServiceType::Other => "OTHER"
        }
    }
}

pub fn log_info(service: LogServiceType, message: String) {
    println!("{} - {} - {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), service.as_str(), message)
}