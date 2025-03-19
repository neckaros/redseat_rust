use chrono::{prelude::*, Duration, DurationRound};
use serde::{Deserialize, Deserializer};

use crate::error::RsResult;
pub static SECONDS_IN_HOUR: u64 = 3600;
pub static SECONDS_IN_DAY: u64 = 86400;

pub fn now() -> DateTime<FixedOffset> {
    Utc::now().fixed_offset()
}

pub type UtcDate = DateTime<Utc>;

pub trait Clock<T> where T: chrono::TimeZone{
    fn print(&self) -> String;
    fn floor_to_hour(&self) -> Option<DateTime<T>>;
    fn add(self, duration: Duration) -> RsResult<DateTime<T>>;
}
impl<T> Clock<T> for DateTime<T> where T: chrono::TimeZone {
    fn print(&self) -> String {
        self.to_rfc3339_opts(SecondsFormat::Secs, true)
    }
    fn floor_to_hour(&self) -> Option<DateTime<T>> {
        T::with_ymd_and_hms(&self.timezone(), self.year(), self.month(), self.day(), self.hour(), 0, 0).single()
    }
    
    fn add(self, duration: Duration) -> RsResult<DateTime<T>> {
        self.checked_add_signed(duration).ok_or(crate::error::Error::TimeCreationError)
    }
}

pub trait RsNaiveDate {
    fn utc(&self) -> RsResult<DateTime<Utc>>;
}

impl RsNaiveDate for NaiveDate {
    fn utc(&self) -> RsResult<DateTime<Utc>>  {
        Ok(Utc.from_local_datetime(&self.and_hms_opt(0, 0, 0).ok_or(crate::Error::TimeCreationError)?).single().ok_or(crate::Error::TimeCreationError)?)
    }
}


pub fn deserialize_optional_date_as_ms_timestamp<'de, D>(
    deserializer: D,
) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    // First deserialize to an Option<String>
    let opt_date_str: Option<String> = Option::deserialize(deserializer)?;
    
    // If the value is null, return None
    match opt_date_str {
        None => Ok(None),
        Some(date_str) => {
            // Parse the date string
            let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                .map_err(serde::de::Error::custom)?;
            
            // Convert to DateTime at midnight UTC
            let datetime = match date.and_hms_opt(0, 0, 0) {
                Some(dt) => dt,
                None => return Err(serde::de::Error::custom("Invalid time"))
            };
            
            // Convert to UTC DateTime
            let utc_datetime = DateTime::<Utc>::from_naive_utc_and_offset(datetime, Utc);
            
            // Get milliseconds timestamp as i64
            let timestamp_ms = utc_datetime.timestamp_millis();
            
            Ok(Some(timestamp_ms))
        }
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

