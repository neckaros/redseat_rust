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
    source: Option<String>,
    name: String,
    description: Option<String>,
    kind: FileTypes,
    mimetype: Option<String>,
    size: Option<usize>,

    added: Option<DateTime<Utc>>,
    modified: Option<DateTime<Utc>>,
    created: Option<DateTime<Utc>>,

    rating: Option<u8>,
    md5: Option<String>,

    width: Option<usize>,
    height: Option<usize>,
    phash: Option<String>,
    thumbhash: Option<String>,
    focal: Option<usize>,
    iso: Option<usize>,
    color_space: Option<String>,
    sspeed: Option<String>,
    orientation: Option<usize>,

    duration: Option<usize>,
    acodecs: Option<Vec<String>>,
    achan: Option<Vec<usize>>,
    vcodecs: Option<Vec<String>>,
    fps: Option<usize>,
    bitrate: Option<usize>,

    long: Option<usize>,
    lat: Option<usize>,
    model: Option<String>,

    pages: Option<usize>,

    progress: Option<usize>,
    tags: Option<Vec<String>>,
    series: Option<Vec<FileEpisode>>,
    people: Option<Vec<String>>,
    thumb: Option<String>,
    thumbv: Option<usize>,
    thumbsize: Option<usize>,
    iv: Option<String>,
    origin: Option<ParsedUrl>,
    movie: Option<String>,
    lang: Option<String>,
    uploader: Option<String>,
    uploadkey: Option<String>,
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