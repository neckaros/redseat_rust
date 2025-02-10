use regex::Regex;
use tokio::process::Command;


pub fn extract_tags(text: &str) -> Vec<String>{
    let re = Regex::new(r"#[\w\d_]+").unwrap();
    re.find_iter(text).map(|h| h.as_str()).map(|h| h.replace("#", "")).collect()
}
pub fn extract_people(text: &str) -> Vec<String>{
    let re = Regex::new(r"@[\w\d_]+").unwrap();
    re.find_iter(text).map(|h| h.as_str()).map(|h| h.replace("@", "")).collect()
}


pub trait Printable {
    fn printable(&self) -> String;
}

impl Printable for Command {
    fn printable(&self) -> String {
        self.as_std().printable()
    }
}

impl Printable for std::process::Command {
    fn printable(&self) -> String {
        let cmd_str = format!(
            "{} {}",
            
            self.get_program().to_string_lossy(),
            self.get_args()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
        );
        cmd_str
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    #[tokio::test]
    async fn people() {

        let text = "Hello @world, this is an @example_123.";
        let people = extract_people(text);
        assert_eq!(people.get(0), Some("world".to_owned()).as_ref());
        assert_eq!(people.get(1), Some("example_123".to_owned()).as_ref());
    }
}