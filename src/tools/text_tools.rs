use regex::Regex;


pub fn extract_tags(text: &str) -> Vec<String>{
    let re = Regex::new(r"#[\w\d_]+").unwrap();
    re.find_iter(text).map(|h| h.as_str()).map(|h| h.replace("#", "")).collect()
}
pub fn extract_people(text: &str) -> Vec<String>{
    let re = Regex::new(r"@[\w\d_]+").unwrap();
    re.find_iter(text).map(|h| h.as_str()).map(|h| h.replace("@", "")).collect()
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