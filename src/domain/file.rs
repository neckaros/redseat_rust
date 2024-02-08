use chrono::{DateTime, Utc};

pub struct FileEpisode {
   id: String,
   season: Option<u8>,
   episode: Option<u8>
}

pub struct ParsedUrl {
    plugin: String,
    platform: String,
    id: String,
    kind: LinkTypes,
    file: Option<String>,
    user: Option<String>
}

pub enum LinkTypes {
    Profile,
    Post
}

pub enum FileTypes {
    Directory,
    Photo,
    Video,
    Archive,
    Album,
    Other
}

pub struct ServerFile {
    id: String,
    pub source: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub kind: FileTypes,
    pub mimetype: Option<String>,
    pub size: Option<usize>,

    pub added: Option<DateTime<Utc>>,
    pub modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,

    pub rating: Option<u8>,
    pub md5: Option<String>,

    pub width: Option<usize>,
    pub height: Option<usize>,
    pub phash: Option<String>,
    pub thumbhash: Option<String>,
    pub focal: Option<usize>,
    pub iso: Option<usize>,
    pub color_space: Option<String>,
    pub sspeed: Option<String>,
    pub orientation: Option<usize>,

    pub duration: Option<usize>,
    pub acodecs: Option<Vec<String>>,
    pub achan: Option<Vec<usize>>,
    pub vcodecs: Option<Vec<String>>,
    pub fps: Option<usize>,
    pub bitrate: Option<usize>,

    pub long: Option<usize>,
    pub lat: Option<usize>,
    pub model: Option<String>,

    pub pages: Option<usize>,

    pub progress: Option<usize>,
    pub tags: Option<Vec<String>>,
    pub series: Option<Vec<FileEpisode>>,
    pub people: Option<Vec<String>>,
    pub thumb: Option<String>,
    pub thumbv: Option<usize>,
    pub thumbsize: Option<usize>,
    pub iv: Option<String>,
    pub origin: Option<ParsedUrl>,
    pub movie: Option<String>,
    pub lang: Option<String>,
    pub uploader: Option<String>,
    pub uploadkey: Option<String>,
} 


pub struct FileForUpdate {
    name: Option<String>,
    description: Option<String>,
    mimetype: Option<String>,
    size: Option<usize>,

    modified: Option<DateTime<Utc>>,
    created: Option<DateTime<Utc>>,

    width: Option<usize>,
    height: Option<usize>,
  
    duration: Option<usize>,
 
    progress: Option<usize>,

    add_tags: Option<Vec<String>>,
    remove_tags: Option<Vec<String>>,

    add_series: Option<Vec<FileEpisode>>,
    remove_series: Option<Vec<FileEpisode>>,

    add_people: Option<Vec<String>>,
    remove_people: Option<Vec<String>>,

    long: Option<usize>,
    lat: Option<usize>,

    origin: Option<ParsedUrl>,
    movie: Option<String>,

    lang: Option<String>,

    uploader: Option<String>,
    uploadkey: Option<String>,
} 