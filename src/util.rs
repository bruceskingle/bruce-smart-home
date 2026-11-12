use std::sync::Arc;

/// An Arc as a component prop, equal only to itself, so that a page is only re-created when it is given a new one.
pub struct SharedProp<T: ?Sized>(pub Arc<T>);

// Not derived, which would need T to be Clone.
impl<T: ?Sized> Clone for SharedProp<T> {
    fn clone(&self) -> Self {
        SharedProp(self.0.clone())
    }
}

impl<T: ?Sized> PartialEq for SharedProp<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub fn as_decimal(value: i32, decimals: usize) -> String {
    // let f = 10 ^ decimals as u32;
    let w = if value < 0 { decimals + 2 } else { decimals + 1 };
    let mut s = format!("{:0width$}", value, width = w);

    let mut l = s.len();

    if l <= decimals {
        while l < decimals {
            s.insert(0, '0');
            l += 1;
        }
        s.insert(0, '.');
        s.insert(0, '0');
    } else {
        s.insert(s.len() - decimals, '.');
    }
    // println!("as_decimal({}, {}) = {}", value, decimals, s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_int() {
        assert_eq!(as_decimal(37364, 2), "373.64");
        assert_eq!(as_decimal(36, 2), "0.36");
        assert_eq!(as_decimal(-36, 2), "-0.36");
    }
}
/// The date and time in the computer's time zone, or UTC if it can not be found, e.g. 2026-10-28 14:32:50
pub fn local_date_time(time: time::OffsetDateTime) -> String {
    use time_tz::OffsetDateTimeExt;
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    match time_tz::system::get_timezone() {
        Ok(tz) => time.to_timezone(tz).format(&format).unwrap_or_default(),
        Err(_) => time.format(&format).unwrap_or_default(),
    }
}
