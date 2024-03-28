use serde::{Deserialize, Serialize};


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FfprobeResult {
    pub streams: Vec<FfprobeStream>,
    pub format: FfprobeFormat,
}


impl FfprobeResult {
	pub fn number_of_video_frames(&self) ->  Option<isize>{
		let video_stream = self.streams.iter().find(|x| x.codec_type == CodecType::Video);
		if let Some(stream) = video_stream {
			stream.number_of_frames()
		} else {
			None
		}
	}

	pub fn duration(&self) ->  Option<f64>{
		let ivalue = self.format.duration.parse::<f64>().ok();
		ivalue
	}
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FfprobeStream {
	pub index: isize,
    pub codec_type: CodecType,
	
	pub nb_frames: Option<String>,
	pub tags: Option<StreamTags>
}

impl FfprobeStream {
	pub fn number_of_frames(&self) ->  Option<isize>{
		if let Some(nb) = &self.nb_frames  {
			let ivalue = nb.parse::<isize>().ok();
			ivalue
		} else if let Some(nb) = self.tags.as_ref().and_then(|t| t.number_of_frames.as_ref())  {
			let ivalue = nb.parse::<isize>().ok();
			ivalue
		} else {
			None
		}
	}
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FfprobeFormat {
	pub duration: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")] 
pub enum CodecType {
	Video,
	Audio,
	Subtitle,
	#[serde(other)]
	Uknown
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct StreamTags {
	#[serde(rename = "NUMBER_OF_FRAMES")]
	number_of_frames: Option<String>,
	#[serde(rename = "DURATION")]
	duration: Option<String>
}