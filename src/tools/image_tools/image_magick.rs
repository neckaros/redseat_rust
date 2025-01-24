use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};


#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMagickInfo {
    pub version: Option<String>,
    pub image: Image,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub name: Option<String>,
    pub base_name: Option<String>,
    pub permissions: Option<i64>,
    pub format: Option<String>,
    pub format_description: Option<String>,
    pub mime_type: Option<String>,
    pub class: Option<String>,
    pub geometry: Geometry,
    pub resolution: Option<Resolution>,
    pub print_size: Option<PrintSize>,
    pub units: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub base_type: Option<String>,
    pub endianness: Option<String>,
    pub colorspace: Option<String>,
    pub depth: Option<i64>,
    pub base_depth: Option<i64>,
    pub channel_depth: Option<ChannelDepth>,
    pub pixels: Option<i64>,
    pub image_statistics: ImageStatistics,
    pub channel_statistics: ChannelStatistics,
    pub rendering_intent: Option<String>,
    pub gamma: Option<f64>,
    pub chromaticity: Chromaticity,
    pub matte_color: Option<String>,
    pub background_color: Option<String>,
    pub border_color: Option<String>,
    pub transparent_color: Option<String>,
    pub interlace: Option<String>,
    pub intensity: Option<String>,
    pub compose: Option<String>,
    pub page_geometry: PageGeometry,
    pub dispose: Option<String>,
    pub iterations: Option<i64>,
    pub compression: Option<String>,
    pub orientation: Option<String>,
    pub properties: Properties,
    pub profiles: Option<Profiles>,
    pub tainted: Option<bool>,
    pub filesize: String,
    pub number_pixels: Option<String>,
    pub pixels_per_second: Option<String>,
    pub user_time: Option<String>,
    pub elapsed_time: Option<String>,
    pub version: Option<String>,
}

impl Image {
    pub fn orientation(&self) -> Option<u8> {
        if let Some(orientation) = self.orientation.as_ref().map(|s| s.as_str()) {
            match orientation {
                "TopLeft" => Some(1),
                "TopRight" => Some(2),
                "BottomRight" => Some(3),
                "BottomLeft" => Some(4),
                "LeftTop" => Some(5),
                "RightTop" => Some(6),
                "RightBottom" => Some(7),
                "LeftBottom" => Some(8),
                _ => None
            }
        } else {
            None
        }
    }

    pub fn f_number(&self) -> Option<f64> {
        if let Some(focals) = &self.properties.exif_fnumber {
            let mut splitted = focals.split('/').map(|s| s.trim());
            let a = splitted.next().and_then(|m| m.parse::<f64>().ok());
            let b = splitted.next().and_then(|m| m.parse::<f64>().ok());
            a.map(|a| a / b.unwrap_or(1.0))
        } else {
            None
        }
    }

    pub fn created(&self) -> Option<i64> {
        if let Some(time) = &self.properties.exif_date_time_original {
            if let Some(date) = Utc.datetime_from_str(time.as_str(), "%Y:%m:%d %H:%M:%S").ok() {
                Some(date.timestamp_millis())
            } else {
                None
            }
                            
        } else {
            None
        }
    }

    pub fn xdim(&self) -> Option<i64> {
        if let Some(dim) = &self.properties.exif_pixel_xdimension {
            dim.parse::<i64>().ok()
        } else {
            None
        }
    }
    pub fn ydim(&self) -> Option<i64> {
        if let Some(dim) = &self.properties.exif_pixel_ydimension {
            dim.parse::<i64>().ok()
        } else {
            None
        }
    }

    pub fn focal(&self) -> Option<u64> {
        if let Some(focals) = &self.properties.exif_focal_length_in35mm_film {
            let mut splitted = focals.split(',').map(|s| s.trim());
            splitted.next().and_then(|m| m.parse::<u64>().ok())
        } else {
            None
        }
    }
    pub fn iso(&self) -> Option<u64> {
        if let Some(focals) = &self.properties.exif_photographic_sensitivity {
            let mut splitted = focals.split(',').map(|s| s.trim());
            splitted.next().and_then(|m| m.parse::<u64>().ok())
        } else {
            None
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub x: Option<i64>,
    pub y: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub x: Option<i64>,
    pub y: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintSize {
    pub x: f64,
    pub y: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDepth {
    pub red: Option<i64>,
    pub green: Option<i64>,
    pub blue: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageStatistics {
    #[serde(rename = "Overall")]
    pub overall: Overall,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overall {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub mean: f64,
    pub median: f64,
    pub standard_deviation: f64,
    pub kurtosis: f64,
    pub skewness: f64,
    pub entropy: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatistics {
    pub red: Red,
    pub green: Green,
    pub blue: Blue,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Red {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub mean: f64,
    pub median: Option<i64>,
    pub standard_deviation: f64,
    pub kurtosis: f64,
    pub skewness: f64,
    pub entropy: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Green {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub mean: f64,
    pub median: Option<i64>,
    pub standard_deviation: f64,
    pub kurtosis: f64,
    pub skewness: f64,
    pub entropy: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blue {
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub mean: f64,
    pub median: Option<i64>,
    pub standard_deviation: f64,
    pub kurtosis: f64,
    pub skewness: f64,
    pub entropy: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chromaticity {
    pub red_primary: RedPrimary,
    pub green_primary: GreenPrimary,
    pub blue_primary: BluePrimary,
    pub white_primary: WhitePrimary,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedPrimary {
    pub x: f64,
    pub y: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GreenPrimary {
    pub x: f64,
    pub y: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BluePrimary {
    pub x: f64,
    pub y: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhitePrimary {
    pub x: f64,
    pub y: f64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGeometry {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub x: Option<i64>,
    pub y: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    #[serde(rename = "date:create")]
    pub date_create: Option<String>,
    #[serde(rename = "date:modify")]
    pub date_modify: Option<String>,
    #[serde(rename = "date:timestamp")]
    pub date_timestamp: Option<String>,
    #[serde(rename = "exif:ApertureValue")]
    pub exif_aperture_value: Option<String>,
    #[serde(rename = "exif:BrightnessValue")]
    pub exif_brightness_value: Option<String>,
    #[serde(rename = "exif:ColorSpace")]
    pub exif_color_space: Option<String>,
    #[serde(rename = "exif:DateTime")]
    pub exif_date_time: Option<String>,
    #[serde(rename = "exif:DateTimeDigitized")]
    pub exif_date_time_digitized: Option<String>,
    #[serde(rename = "exif:DateTimeOriginal")]
    pub exif_date_time_original: Option<String>,
    #[serde(rename = "exif:ExifOffset")]
    pub exif_exif_offset: Option<String>,
    #[serde(rename = "exif:ExifVersion")]
    pub exif_exif_version: Option<String>,
    #[serde(rename = "exif:ExposureBiasValue")]
    pub exif_exposure_bias_value: Option<String>,
    #[serde(rename = "exif:ExposureMode")]
    pub exif_exposure_mode: Option<String>,
    #[serde(rename = "exif:ExposureProgram")]
    pub exif_exposure_program: Option<String>,
    #[serde(rename = "exif:ExposureTime")]
    pub exif_exposure_time: Option<String>,
    #[serde(rename = "exif:Flash")]
    pub exif_flash: Option<String>,
    #[serde(rename = "exif:FNumber")]
    pub exif_fnumber: Option<String>,
    #[serde(rename = "exif:FocalLength")]
    pub exif_focal_length: Option<String>,
    #[serde(rename = "exif:FocalLengthIn35mmFilm")]
    pub exif_focal_length_in35mm_film: Option<String>,
    #[serde(rename = "exif:GPSAltitude")]
    pub exif_gpsaltitude: Option<String>,
    #[serde(rename = "exif:GPSAltitudeRef")]
    pub exif_gpsaltitude_ref: Option<String>,
    #[serde(rename = "exif:GPSDateStamp")]
    pub exif_gpsdate_stamp: Option<String>,
    #[serde(rename = "exif:GPSDestBearing")]
    pub exif_gpsdest_bearing: Option<String>,
    #[serde(rename = "exif:GPSDestBearingRef")]
    pub exif_gpsdest_bearing_ref: Option<String>,
    #[serde(rename = "exif:GPSHPositioningError")]
    pub exif_gpshpositioning_error: Option<String>,
    #[serde(rename = "exif:GPSImgDirection")]
    pub exif_gpsimg_direction: Option<String>,
    #[serde(rename = "exif:GPSImgDirectionRef")]
    pub exif_gpsimg_direction_ref: Option<String>,
    #[serde(rename = "exif:GPSInfo")]
    pub exif_gpsinfo: Option<String>,
    #[serde(rename = "exif:GPSLatitude")]
    pub exif_gpslatitude: Option<String>,
    #[serde(rename = "exif:GPSLatitudeRef")]
    pub exif_gpslatitude_ref: Option<String>,
    #[serde(rename = "exif:GPSLongitude")]
    pub exif_gpslongitude: Option<String>,
    #[serde(rename = "exif:GPSLongitudeRef")]
    pub exif_gpslongitude_ref: Option<String>,
    #[serde(rename = "exif:GPSSpeed")]
    pub exif_gpsspeed: Option<String>,
    #[serde(rename = "exif:GPSSpeedRef")]
    pub exif_gpsspeed_ref: Option<String>,
    #[serde(rename = "exif:GPSTimeStamp")]
    pub exif_gpstime_stamp: Option<String>,
    #[serde(rename = "exif:LensMake")]
    pub exif_lens_make: Option<String>,
    #[serde(rename = "exif:LensModel")]
    pub exif_lens_model: Option<String>,
    #[serde(rename = "exif:LensSpecification")]
    pub exif_lens_specification: Option<String>,
    #[serde(rename = "exif:Make")]
    pub exif_make: Option<String>,
    #[serde(rename = "exif:MakerNote")]
    pub exif_maker_note: Option<String>,
    #[serde(rename = "exif:MeteringMode")]
    pub exif_metering_mode: Option<String>,
    #[serde(rename = "exif:Model")]
    pub exif_model: Option<String>,
    #[serde(rename = "exif:OffsetTime")]
    pub exif_offset_time: Option<String>,
    #[serde(rename = "exif:OffsetTimeDigitized")]
    pub exif_offset_time_digitized: Option<String>,
    #[serde(rename = "exif:OffsetTimeOriginal")]
    pub exif_offset_time_original: Option<String>,
    #[serde(rename = "exif:PhotographicSensitivity")]
    pub exif_photographic_sensitivity: Option<String>,
    #[serde(rename = "exif:PixelXDimension")]
    pub exif_pixel_xdimension: Option<String>,
    #[serde(rename = "exif:PixelYDimension")]
    pub exif_pixel_ydimension: Option<String>,
    #[serde(rename = "exif:SceneType")]
    pub exif_scene_type: Option<String>,
    #[serde(rename = "exif:SensingMethod")]
    pub exif_sensing_method: Option<String>,
    #[serde(rename = "exif:ShutterSpeedValue")]
    pub exif_shutter_speed_value: Option<String>,
    #[serde(rename = "exif:Software")]
    pub exif_software: Option<String>,
    #[serde(rename = "exif:SubjectArea")]
    pub exif_subject_area: Option<String>,
    #[serde(rename = "exif:SubSecTimeDigitized")]
    pub exif_sub_sec_time_digitized: Option<String>,
    #[serde(rename = "exif:SubSecTimeOriginal")]
    pub exif_sub_sec_time_original: Option<String>,
    #[serde(rename = "exif:WhiteBalance")]
    pub exif_white_balance: Option<String>,
    #[serde(rename = "icc:copyright")]
    pub icc_copyright: Option<String>,
    #[serde(rename = "icc:description")]
    pub icc_description: Option<String>,
    pub signature: Option<String>,
    pub unknown: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profiles {
    pub exif: Option<Exif>,
    pub icc: Option<Icc>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exif {
    pub length: Option<i64>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Icc {
    pub length: Option<i64>,
}
