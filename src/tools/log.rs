use chrono::Local;

pub enum LogServiceType {
    Register,
    LibraryCreation,
    Database,
    Scoket,
    Plugin,
    Scheduler,
    Source,
    Other
}
impl LogServiceType {
    fn as_str(&self) -> &'static str {
        match self {
            LogServiceType::Register => "REGISTER",
            LogServiceType::LibraryCreation => "LIRARY_CREATION",
            LogServiceType::Database => "DATABASE",
            LogServiceType::Scoket => "SOCKET",
            LogServiceType::Plugin => "PLUGIN",
            LogServiceType::Source => "SOURCE",
            LogServiceType::Scheduler => "SCHEDULER",
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

pub fn log_warn(service: LogServiceType, message: String) {
    println!("{} - WARN - {} - {}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f"), service.as_str(), message)
}