use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub const QUALITY_ORDER: &[&str] = &["4K", "FHD", "HEVC", "HD", "SD", "FHD BKP", "HD BKP", "LOW"];

lazy_static! {
    static ref RE_COUNTRY_PREFIX: Regex = Regex::new(r"^\|[A-Z]+\|\s*").unwrap();
    static ref RE_QUALITY_SUFFIX: Regex = Regex::new(r"(?i)\s*(4K\s*HDR?\s*(UHD)?|UHD|FHD\+*|FULL\s*HD|FHD|HD|SD|LOW|HEVC|H265)(\s*BKP)?\s*(\(.*?\))?\s*$").unwrap();
    static ref RE_SEASON_EPISODE: Regex = Regex::new(r"S(\d+)\s*E(\d+)").unwrap();
    static ref RE_SEASON_EPISODE_TAIL: Regex = Regex::new(r"\s*S\d+\s*E\d+.*$").unwrap();
    static ref RE_TRAILING_YEAR: Regex = Regex::new(r"\s+(\d{4})\s*$").unwrap();
    static ref RE_LANG_TAG: Regex = Regex::new(r"(?i)\s*\((?:MULTI|VF|VOSTFR|FR|EN)\)\s*").unwrap();
    static ref RE_QUALITY_TAG: Regex = Regex::new(r"(?i)\s*(4K|UHD|FHD|HD|SD|LOW|HEVC|H265)(\s*BKP)?\s*$").unwrap();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct M3uHeader {
    pub url_tvg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct M3uEntry {
    pub tvg_id: Option<String>,
    pub tvg_name: Option<String>,
    pub tvg_logo: Option<String>,
    pub group_title: Option<String>,
    pub display_name: String,
    pub url: String,
    pub duration: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum M3uContentType {
    Live,
    Vod,
    Series,
}

impl M3uEntry {
    pub fn content_type(&self) -> M3uContentType {
        if self.url.contains("/movie/") {
            M3uContentType::Vod
        } else if self.url.contains("/series/") {
            M3uContentType::Series
        } else {
            M3uContentType::Live
        }
    }

    pub fn quality(&self) -> Option<String> {
        let name = self.tvg_name.as_deref().unwrap_or(&self.display_name);
        let upper = name.to_uppercase();
        let is_bkp = upper.contains(" BKP");
        if upper.contains("4K") || upper.contains("UHD") {
            Some("4K".to_string())
        } else if upper.contains("HEVC") || upper.contains("H265") {
            Some("HEVC".to_string())
        } else if upper.contains("FHD") || upper.contains("FULL HD") {
            Some(if is_bkp { "FHD BKP" } else { "FHD" }.to_string())
        } else if upper.contains(" HD") {
            Some(if is_bkp { "HD BKP" } else { "HD" }.to_string())
        } else if upper.contains(" SD") {
            Some("SD".to_string())
        } else if upper.contains(" LOW") {
            Some("LOW".to_string())
        } else {
            None
        }
    }

    /// Returns a display-friendly variant name by stripping the country prefix (e.g., `|FR|`).
    pub fn variant_name(&self) -> String {
        let name = self.tvg_name.as_deref().unwrap_or(&self.display_name);
        RE_COUNTRY_PREFIX.replace(name, "").trim().to_string()
    }

    /// Returns a cleaned channel name for grouping variants of the same channel.
    /// Strips quality suffixes, country prefixes like `|FR|`, and normalizes whitespace.
    pub fn channel_key(&self) -> String {
        let name = self.tvg_name.as_deref().unwrap_or(&self.display_name);
        let mut cleaned = name.to_string();

        cleaned = RE_COUNTRY_PREFIX.replace(&cleaned, "").to_string();
        cleaned = RE_QUALITY_SUFFIX.replace(&cleaned, "").to_string();

        cleaned.trim().to_string()
    }

    /// Parse season and episode from series entry names like "Show Name S01 E02"
    pub fn parse_season_episode(&self) -> Option<(u32, u32)> {
        let name = self.tvg_name.as_deref().unwrap_or(&self.display_name);
        RE_SEASON_EPISODE.captures(name).and_then(|caps| {
            let season = caps.get(1)?.as_str().parse().ok()?;
            let episode = caps.get(2)?.as_str().parse().ok()?;
            Some((season, episode))
        })
    }

    /// Parse movie/series name and year from titles like "Movie Name (MULTI) FHD 2026"
    pub fn parse_name_and_year(&self) -> (String, Option<u32>) {
        let name = self.tvg_name.as_deref().unwrap_or(&self.display_name);
        let mut cleaned = name.to_string();

        cleaned = RE_SEASON_EPISODE_TAIL.replace(&cleaned, "").to_string();

        let year = RE_TRAILING_YEAR.captures(&cleaned).and_then(|caps| {
            let y: u32 = caps.get(1)?.as_str().parse().ok()?;
            if y >= 1900 && y <= 2100 {
                Some(y)
            } else {
                None
            }
        });

        if year.is_some() {
            cleaned = RE_TRAILING_YEAR.replace(&cleaned, "").to_string();
        }

        cleaned = RE_LANG_TAG.replace_all(&cleaned, " ").to_string();
        cleaned = RE_QUALITY_TAG.replace(&cleaned, "").to_string();

        (cleaned.trim().to_string(), year)
    }

    /// Parse series name only (without S01 E02 and quality info)
    pub fn parse_series_name(&self) -> String {
        let (name, _) = self.parse_name_and_year();
        name
    }
}

#[derive(Debug)]
pub struct M3uParseResult {
    pub header: M3uHeader,
    pub entries: Vec<M3uEntry>,
}

/// Parse an M3U Plus format string into header + entries.
pub fn parse_m3u(content: &str) -> M3uParseResult {
    let mut header = M3uHeader { url_tvg: None };
    let mut entries = Vec::new();
    let mut current_extinf: Option<(i64, String)> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("#EXTM3U") {
            // Parse header attributes
            if let Some(url) = extract_attribute(line, "url-tvg") {
                header.url_tvg = Some(url);
            }
            continue;
        }

        if line.starts_with("#EXTINF:") {
            // Parse duration and attributes
            let after_prefix = &line[8..];
            let duration = parse_duration(after_prefix);
            current_extinf = Some((duration, line.to_string()));
            continue;
        }

        if line.starts_with('#') {
            continue;
        }

        // This is a URL line
        if let Some((duration, extinf_line)) = current_extinf.take() {
            let tvg_id = extract_attribute(&extinf_line, "tvg-id").filter(|s| !s.is_empty());
            let tvg_name = extract_attribute(&extinf_line, "tvg-name").filter(|s| !s.is_empty());
            let tvg_logo = extract_attribute(&extinf_line, "tvg-logo").filter(|s| !s.is_empty());
            let group_title = extract_attribute(&extinf_line, "group-title").filter(|s| !s.is_empty());

            // Display name is the text after the last comma in the EXTINF line
            let display_name = extinf_line
                .rfind(',')
                .map(|i| extinf_line[i + 1..].trim().to_string())
                .unwrap_or_default();

            entries.push(M3uEntry {
                tvg_id,
                tvg_name,
                tvg_logo,
                group_title,
                display_name,
                url: line.to_string(),
                duration,
            });
        }
    }

    M3uParseResult { header, entries }
}

fn parse_duration(s: &str) -> i64 {
    // Duration is the number before the first space or attribute
    let end = s.find(|c: char| c == ' ' || c == '\t').unwrap_or(s.len());
    s[..end].parse().unwrap_or(-1)
}

fn extract_attribute(line: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    let start = line.find(&pattern)?;
    let value_start = start + pattern.len();
    let rest = &line[value_start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"#EXTM3U
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 SD" tvg-logo="https://i.imgur.com/LMxTAzY.png" group-title="FR TV SD (FRANCE)",|FR| TF1 SD
http://host:80/user/pass/12071
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 HD" tvg-logo="https://i.imgur.com/LMxTAzY.png" group-title="FR TV HD (France)",|FR| TF1 HD
http://host:80/user/pass/1
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 FHD" tvg-logo="https://i.imgur.com/LMxTAzY.png" group-title="FR TV FULL HD|4K  (France)",|FR| TF1 FHD
http://host:80/user/pass/1149
#EXTINF:-1 tvg-id="" tvg-name="|FR| TF1 4K HDR UHD (Résolution Exclus)" tvg-logo="https://i.imgur.com/UlG9dS2.png" group-title="FR TV FULL HD|4K  (France)",|FR| TF1 4K HDR UHD (Résolution Exclus)
http://host:80/user/pass/37744
#EXTINF:-1 tvg-id="TF1.fr" tvg-name="|FR| TF1 LOW" tvg-logo="https://i.imgur.com/LMxTAzY.png" group-title="FR TV LOW (France)",|FR| TF1 LOW
http://host:80/user/pass/99999
#EXTINF:-1 tvg-id="beinSports1.fr" tvg-name="|FR| BEIN SPORTS 1 HEVC" tvg-logo="" group-title="FR SPORTS",|FR| BEIN SPORTS 1 HEVC
http://host:80/user/pass/88888
#EXTINF:-1 tvg-id="beinSports1.fr" tvg-name="|FR| BEIN SPORTS 1 HD BKP" tvg-logo="" group-title="FR SPORTS",|FR| BEIN SPORTS 1 HD BKP
http://host:80/user/pass/77777
#EXTINF:-1 tvg-id="RMCSport1.fr" tvg-name="|FR| RMC SPORT 1 FHD BKP" tvg-logo="" group-title="FR SPORTS",|FR| RMC SPORT 1 FHD BKP
http://host:80/user/pass/66666
#EXTINF:-1 tvg-id="" tvg-name="Killer Whale (MULTI) FHD 2026" tvg-logo="https://image.tmdb.org/t/p/w600_and_h900_bestv2/xC6zdIoIHjhOIFmjNyGgtzhuhiF.jpg" group-title="FILMS RÉCEMMENT AJOUTÉS",Killer Whale (MULTI) FHD 2026
http://host:80/movie/user/pass/197831.mkv
#EXTINF:-1 tvg-id="" tvg-name="Le Monde incroyable de Gumball (MULTI) FHD S01 E02" tvg-logo="https://image.tmdb.org/t/p/w185/pVpRzjI9lA8M0SzaHVE6bJWL9wE.jpg" group-title="ANIMATION",Le Monde incroyable de Gumball (MULTI) FHD S01 E02
http://host:80/series/user/pass/197898.mkv"#;

    #[test]
    fn parse_basic_m3u() {
        let result = parse_m3u(SAMPLE);
        assert_eq!(result.entries.len(), 10);
    }

    #[test]
    fn content_type_detection() {
        let result = parse_m3u(SAMPLE);
        // First 4 are live channels
        assert_eq!(result.entries[0].content_type(), M3uContentType::Live);
        assert_eq!(result.entries[1].content_type(), M3uContentType::Live);
        // Movie
        assert_eq!(result.entries[8].content_type(), M3uContentType::Vod);
        // Series
        assert_eq!(result.entries[9].content_type(), M3uContentType::Series);
    }

    #[test]
    fn quality_extraction() {
        let result = parse_m3u(SAMPLE);
        assert_eq!(result.entries[0].quality(), Some("SD".to_string()));
        assert_eq!(result.entries[1].quality(), Some("HD".to_string()));
        assert_eq!(result.entries[2].quality(), Some("FHD".to_string()));
        assert_eq!(result.entries[3].quality(), Some("4K".to_string()));
        assert_eq!(result.entries[4].quality(), Some("LOW".to_string()));
        assert_eq!(result.entries[5].quality(), Some("HEVC".to_string()));
        assert_eq!(result.entries[6].quality(), Some("HD BKP".to_string()));
        assert_eq!(result.entries[7].quality(), Some("FHD BKP".to_string()));
    }

    #[test]
    fn channel_key_grouping() {
        let result = parse_m3u(SAMPLE);
        // All TF1 variants should produce the same channel key
        let key0 = result.entries[0].channel_key();
        let key1 = result.entries[1].channel_key();
        let key2 = result.entries[2].channel_key();
        let key_low = result.entries[4].channel_key();
        assert_eq!(key0, "TF1");
        assert_eq!(key1, "TF1");
        assert_eq!(key2, "TF1");
        assert_eq!(key_low, "TF1");

        // HEVC and BKP variants should also strip quality
        assert_eq!(result.entries[5].channel_key(), "BEIN SPORTS 1");
        assert_eq!(result.entries[6].channel_key(), "BEIN SPORTS 1");
        assert_eq!(result.entries[7].channel_key(), "RMC SPORT 1");
    }

    #[test]
    fn season_episode_parsing() {
        let result = parse_m3u(SAMPLE);
        let series = &result.entries[9];
        assert_eq!(series.parse_season_episode(), Some((1, 2)));
    }

    #[test]
    fn movie_name_and_year() {
        let result = parse_m3u(SAMPLE);
        let movie = &result.entries[8];
        let (name, year) = movie.parse_name_and_year();
        assert_eq!(name, "Killer Whale");
        assert_eq!(year, Some(2026));
    }

    #[test]
    fn series_name_parsing() {
        let result = parse_m3u(SAMPLE);
        let series = &result.entries[9];
        let name = series.parse_series_name();
        assert_eq!(name, "Le Monde incroyable de Gumball");
    }

    #[test]
    fn attributes_parsing() {
        let result = parse_m3u(SAMPLE);
        let entry = &result.entries[0];
        assert_eq!(entry.tvg_id, Some("TF1.fr".to_string()));
        assert_eq!(entry.tvg_name, Some("|FR| TF1 SD".to_string()));
        assert_eq!(entry.tvg_logo, Some("https://i.imgur.com/LMxTAzY.png".to_string()));
        assert_eq!(entry.group_title, Some("FR TV SD (FRANCE)".to_string()));
        assert_eq!(entry.display_name, "|FR| TF1 SD");
        assert_eq!(entry.url, "http://host:80/user/pass/12071");
        assert_eq!(entry.duration, -1);
    }

    #[test]
    fn empty_tvg_id_becomes_none() {
        let result = parse_m3u(SAMPLE);
        // Entry with tvg-id="" should be None
        assert_eq!(result.entries[3].tvg_id, None);
    }

    #[test]
    fn quality_low_with_parenthetical() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-id="" tvg-name="|LIGUE 1+| MATCH DU DIMANCHE SOIR LOW (MULTI AUDIO StadiumFX)" tvg-logo="" group-title="FR SPORTS",|LIGUE 1+| MATCH DU DIMANCHE SOIR LOW (MULTI AUDIO StadiumFX)
http://host:80/user/pass/55555"#;
        let result = parse_m3u(m3u);
        let entry = &result.entries[0];
        assert_eq!(entry.quality(), Some("LOW".to_string()));
        // |LIGUE 1+| is not a simple country prefix, so it stays
        assert_eq!(entry.channel_key(), "|LIGUE 1+| MATCH DU DIMANCHE SOIR");
    }

    #[test]
    fn h265_quality_detection() {
        let m3u = r#"#EXTM3U
#EXTINF:-1 tvg-id="CANALplus.fr" tvg-name="|FR| CANAL+  H265" tvg-logo="" group-title="FR TV HD",|FR| CANAL+  H265
http://host:80/user/pass/7183"#;
        let result = parse_m3u(m3u);
        let entry = &result.entries[0];
        assert_eq!(entry.quality(), Some("HEVC".to_string()));
        assert_eq!(entry.channel_key(), "CANAL+");
    }
}
