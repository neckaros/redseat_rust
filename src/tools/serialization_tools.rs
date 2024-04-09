use serde::{Serialize, Serializer};


pub fn rating_serializer<S: Serializer>(v: &Option<f32>, serializer: S) -> Result<S::Ok, S::Error> {
    let v = v.as_ref().and_then(|v| Some((*v as f64 * 100.0).trunc() / 100.0));
    Option::<f64>::serialize(&v, serializer)
}