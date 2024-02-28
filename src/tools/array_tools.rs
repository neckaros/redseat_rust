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