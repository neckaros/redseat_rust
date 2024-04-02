pub fn replace_add_remove_from_array<T: PartialEq + Clone>(existing: Option<Vec<T>>, replace: Option<Vec<T>>, add: Option<Vec<T>>, remove: Option<Vec<T>>) -> Option<Vec<T>> {

    let existing = match replace {
        Some(a) => Some(a),
        None => existing,
    };
    add_remove_from_array(existing, add, remove)
}

pub fn add_remove_from_array<T: PartialEq + Clone>(existing: Option<Vec<T>>, add: Option<Vec<T>>, remove: Option<Vec<T>>) -> Option<Vec<T>> {
    let alts = if let Some(add_alts) = add {
        if let Some(mut existing_alts) = existing {
            for alt in add_alts {
                if !existing_alts.contains(&alt) {
                    existing_alts.push(alt);
                }
            }
            
            Some(existing_alts)
        } else {
            Some(add_alts)
        }
    } else {
       existing
    };
   let alts =  if let Some(remove_alts) = remove  {
        let mut base = match alts {
            Some(alts) => Some(alts),
            None => alts,
        };
        if let Some(existing_alts) = base.as_mut() {
            for alt in remove_alts {
                if let Some(index) = existing_alts.iter().position(|x| *x == alt) {
                    existing_alts.swap_remove(index);
                }
            }
        } 
        base
    } else {
        alts
    };


    alts
}

pub trait AddOrSetArray<T> where T: PartialEq {
    fn add_or_set(&mut self, add: Vec<T>);
}

impl<T: PartialEq> AddOrSetArray<T> for Option<Vec<T>> {
    fn add_or_set(&mut self, add:  Vec<T>) {
        if let Some(existing) = self {
            for n in add {
                if !existing.contains(&n) {
                    existing.push(n);
                }
            }
            //existing.append(&mut add);
        } else {
            let new_value = Some(add);
            *self = new_value;
        }
    }
}


#[cfg(test)]
mod tests {
    use crate::domain::media::{MediaForUpdate, MediaItemReference};

    use super::*;


    #[tokio::test]
    async fn add_or_set_test() {
        let add = vec![MediaItemReference {
            id: "test".to_owned(),
            conf: None
        }];
        let mut media_update = MediaForUpdate {
            add_tags: None,
            ..Default::default()
        };
        media_update.add_tags.add_or_set(add);

        assert_eq!(media_update.add_tags.as_ref().unwrap().len(), 1);
        assert_eq!(media_update.add_tags.unwrap().get(0).unwrap().id, "test".to_owned());
    }
    #[tokio::test]
    async fn add_or_set_test_with_value() {
        let add = vec![MediaItemReference {
            id: "test".to_owned(),
            conf: None
        }];
        let mut media_update = MediaForUpdate {
            add_tags: Some(vec![MediaItemReference {
                id: "exist".to_owned(),
                conf: None
            }]),
            ..Default::default()
        };
        media_update.add_tags.add_or_set(add);

        assert_eq!(media_update.add_tags.as_ref().unwrap().len(), 2);
        assert_eq!(media_update.add_tags.as_ref().unwrap().get(0).unwrap().id, "exist".to_owned());
        assert_eq!(media_update.add_tags.as_ref().unwrap().get(1).unwrap().id, "test".to_owned());
    }

    #[tokio::test]
    async fn add_or_set_test_with_existing() {
        let add = vec![MediaItemReference {
            id: "exist".to_owned(),
            conf: None
        }];
        let mut media_update = MediaForUpdate {
            add_tags: Some(vec![MediaItemReference {
                id: "exist".to_owned(),
                conf: None
            }]),
            ..Default::default()
        };
        media_update.add_tags.add_or_set(add);

        assert_eq!(media_update.add_tags.as_ref().unwrap().len(), 1);
        assert_eq!(media_update.add_tags.as_ref().unwrap().get(0).unwrap().id, "exist".to_owned());
    }
}