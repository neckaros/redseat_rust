use chrono::{prelude::*, Duration, DurationRound};

use crate::error::RsResult;
pub static SECONDS_IN_HOUR: u64 = 3600;
pub static SECONDS_IN_DAY: u64 = 86400;

pub fn now() -> DateTime<FixedOffset> {
    Utc::now().fixed_offset()
}

pub type UtcDate = DateTime<Utc>;

pub trait Clock {
    fn print(&self) -> String;
    fn floor_to_hour(&self) -> Option<DateTime<FixedOffset>>;
    fn add(self, duration: Duration) -> RsResult<DateTime<FixedOffset>>;
}



impl Clock for DateTime<FixedOffset> {
    fn print(&self) -> String {
        self.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
    fn floor_to_hour(&self) -> Option<DateTime<FixedOffset>> {
        Utc.with_ymd_and_hms(self.year(), self.month(), self.day(), self.hour(), 0, 0).single().and_then(|r| Some(r.fixed_offset()))
    }
    
    fn add(self, duration: Duration) -> RsResult<DateTime<FixedOffset>> {
        self.checked_add_signed(duration).ok_or(crate::error::Error::TimeCreationError)
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::library::LibraryRole;

    use super::*;

    #[test]
    fn test_role() {
        println!("UTC: {}", now().print());
        println!("UTC: {}", now().floor_to_hour().unwrap().print());
        
        println!("UTC: {}", now().add(Duration::days(2)).unwrap().print());
        
        println!("UTC: {}", now().checked_sub_signed(Duration::days(2)).unwrap().print());

    }
}