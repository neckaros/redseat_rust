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

	pub fn size(&self) ->  (Option<u32>, Option<u32>){
		let video_stream = self.streams.iter().find(|x| x.codec_type == CodecType::Video);
		if let Some(stream) = video_stream {
			(stream.width, stream.height)
		} else {
			(None, None)
		}
	}

	pub fn video_stream(&self) ->  Option<&FfprobeStream> {
		self.streams.iter().find(|x| x.codec_type == CodecType::Video)
	}


	pub fn duration(&self) ->  Option<f64>{
		let ivalue = self.format.duration.parse::<f64>().ok();
		ivalue
	}
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FfprobeStream {
	pub index: isize,
	pub codec_name: Option<String>,
    pub codec_type: CodecType,

	pub width: Option<u32>,
	pub height: Option<u32>,
	pub color_space: Option<String>,
	pub bit_rate: Option<String>,
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

	
	pub fn bitrate(&self) ->  Option<u64>{
		let ivalue = self.bit_rate.as_ref().and_then(|b| b.parse::<u64>().ok());
		ivalue
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