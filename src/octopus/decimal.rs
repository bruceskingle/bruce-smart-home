use serde::de::{self, Visitor};
use serde::{Deserializer, Serialize, Serializer};
use std::fmt::{self, Display};
use std::hash::Hash;
use std::ops::{Add, AddAssign, Deref, Div, DivAssign, Mul, MulAssign, Sub, SubAssign};
use std::str::FromStr;

use bruce_graphql::types::Int;

/*
Implementation of the Decimal CustomScalar defined in the Octopus graphql API Schema.
=====================================================================================

The schema simply says

  # The `Decimal` scalar type represents a python Decimal.

This implementation is a wrapper around a rust_decimal::Decimal which provides serde serialization to/from a String.

*/

#[derive(Debug, Clone, Copy)]
pub struct Decimal(rust_decimal::Decimal);

impl Decimal {
    pub const ZERO: Decimal = Decimal(rust_decimal::Decimal::ZERO);

    pub fn new(num: i64, scale: u32) -> Decimal {
        Decimal(rust_decimal::Decimal::new(num, scale))
    }

    pub fn from_int(i: &Int) -> Decimal {
        Decimal::new(**i as i64, 0)
    }

    pub fn is_positive(&self) -> bool {
        self.0.gt(&rust_decimal::Decimal::from(0)) //.gt(&0)
    }

    pub fn is_non_zero(&self) -> bool {
        self.0.ne(&rust_decimal::Decimal::from(0)) //.gt(&0)
    }

    pub fn round_dp(&self, dp: u32) -> Decimal {
        Decimal(self.0.round_dp(dp))
    }

    pub fn round_dp_with_strategy(&self, dp: u32, strategy: rust_decimal::RoundingStrategy) -> Decimal {
        Decimal(self.0.round_dp_with_strategy(dp, strategy))
    }

    pub fn normalize(&self) -> Decimal {
        Decimal(self.0.normalize())
    }
}

// impl Div for Decimal {
//     type Output = Decimal;

//     fn div(self, rhs: Self) -> Self::Output {
//         Decimal(self.0.div(rhs.0))
//     }
// }

// impl Mul for Decimal {
//     type Output = Decimal;

//     fn mul(self, rhs: Self) -> Self::Output {
//         Decimal(self.0.mul(rhs.0))
//     }
// }

// impl AddAssign for Decimal {
//     fn add_assign(&mut self, rhs: Self) {
//         self.0.add_assign(rhs.0);
//     }
// }

// impl Add for Decimal {
//     type Output = Decimal;

//     fn add(self, rhs: Self) -> Self::Output {
//         Decimal(self.0.add(rhs.0))
//     }
// }

// impl Sub for Decimal {
//     type Output = Decimal;

//     fn sub(self, rhs: Self) -> Self::Output {
//         Decimal(self.0.sub(rhs.0))
//     }
// }

macro_rules! forward_ref_binary_op {
    ($imp:ident, $method:ident) => {
        impl $imp<&Decimal> for Decimal {
            type Output = Decimal;
            fn $method(self, rhs: &Decimal) -> Decimal {
                $imp::$method(self, *rhs)
            }
        }
        impl $imp<Decimal> for &Decimal {
            type Output = Decimal;
            fn $method(self, rhs: Decimal) -> Decimal {
                $imp::$method(*self, rhs)
            }
        }
        impl $imp<&Decimal> for &Decimal {
            type Output = Decimal;
            fn $method(self, rhs: &Decimal) -> Decimal {
                $imp::$method(*self, *rhs)
            }
        }
    };
}

macro_rules! forward_ref_assign_op {
    ($imp:ident, $method:ident) => {
        impl $imp<&Decimal> for Decimal {
            fn $method(&mut self, rhs: &Decimal) {
                $imp::$method(self, *rhs)
            }
        }
    };
}

impl Div for Decimal {
    type Output = Decimal;
    fn div(self, rhs: Self) -> Self::Output {
        Decimal(self.0.div(rhs.0))
    }
}
forward_ref_binary_op!(Div, div);

impl Mul for Decimal {
    type Output = Decimal;
    fn mul(self, rhs: Self) -> Self::Output {
        Decimal(self.0.mul(rhs.0))
    }
}
forward_ref_binary_op!(Mul, mul);

impl Add for Decimal {
    type Output = Decimal;
    fn add(self, rhs: Self) -> Self::Output {
        Decimal(self.0.add(rhs.0))
    }
}
forward_ref_binary_op!(Add, add);

impl Sub for Decimal {
    type Output = Decimal;
    fn sub(self, rhs: Self) -> Self::Output {
        Decimal(self.0.sub(rhs.0))
    }
}
forward_ref_binary_op!(Sub, sub);

impl AddAssign for Decimal {
    fn add_assign(&mut self, rhs: Self) {
        self.0.add_assign(rhs.0);
    }
}
forward_ref_assign_op!(AddAssign, add_assign);

impl SubAssign for Decimal {
    fn sub_assign(&mut self, rhs: Self) {
        self.0.sub_assign(rhs.0);
    }
}
forward_ref_assign_op!(SubAssign, sub_assign);

impl MulAssign for Decimal {
    fn mul_assign(&mut self, rhs: Self) {
        self.0.mul_assign(rhs.0);
    }
}
forward_ref_assign_op!(MulAssign, mul_assign);

impl DivAssign for Decimal {
    fn div_assign(&mut self, rhs: Self) {
        self.0.div_assign(rhs.0);
    }
}
forward_ref_assign_op!(DivAssign, div_assign);

impl Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // f.pad(&format!("{}.{}", self.int, self.dec))
        self.0.fmt(f)
    }
}

impl Deref for Decimal {
    type Target = rust_decimal::Decimal;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<i32> for Decimal {
    fn from(value: i32) -> Self {
        Decimal(rust_decimal::Decimal::from(value))
    }
}

impl FromStr for Decimal {
    type Err = anyhow::Error;

    fn from_str(str: &str) -> Result<Decimal, Self::Err> {
        // Python Decimals may be serialized in scientific notation, e.g. "0E-18"
        if str.contains(['e', 'E']) {
            Ok(Decimal(rust_decimal::Decimal::from_scientific(str)?))
        } else {
            Ok(Decimal(rust_decimal::Decimal::from_str(str)?))
        }
        // let mut int: i32 = 0;
        // let mut dec: u32 = 0;
        // let mut i = 0;

        // for part in str.split(".") {
        //   if i==0 {
        //     int = i32::from_str(part)?
        //   }
        //   else if i==1 {
        //     dec = u32::from_str(part)?
        //   }
        //   else {
        //     return Err(Error::StringError(format!("Too many decimal points '{}'", str)))
        //   }
        //   i += 1;
        // }

        // if i==0 {
        //   return Err(Error::StringError(format!("Empty value ''{}'", str)))
        // }

        // Ok(Decimal {
        //   int,
        //   dec
        // })
    }
}

impl Eq for Decimal {}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.0.cmp(&other.0))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl Hash for Decimal {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

struct DecimalVisitor;

impl<'de> Visitor<'de> for DecimalVisitor {
    type Value = Decimal;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a decimal value")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        // Trailing zeros after the decimal point are removed, e.g. "1.500" becomes 1.5
        match Decimal::from_str(value) {
            Ok(value) => Ok(value.normalize()),
            Err(error) => Err(E::custom(format!("Invalid Decimal value: {}", error))),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Decimal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_string(DecimalVisitor)
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // serializer.serialize_str(&format!("{}.{}", self.int, self.dec))
        serializer.serialize_str(&self.0.to_string())
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, Debug)]
    struct MyStruct {
        decimal: Decimal,
    }

    #[test]
    fn test_from_str() {
        let dd = rust_decimal::Decimal::new(123456, 3);
        assert_eq!(dd.to_string(), "123.456");

        let decimal = Decimal::from_str("123.456").unwrap();

        assert_eq!(decimal.0, rust_decimal::Decimal::new(123456, 3));
    }

    fn expect_parse(s: &str, v: rust_decimal::Decimal) {
        let result: Result<MyStruct, serde_json::Error> = serde_json::from_str(s);
        if let Ok(value) = result {
            assert_eq!(value.decimal.0, v);
        } else {
            panic!("Expecting {} for {}", v, s);
        }
    }

    fn expect_parse_error(s: &str) {
        let result: Result<MyStruct, serde_json::Error> = serde_json::from_str(s);
        if let Ok(v) = result {
            panic!("Expecting error for {}, got {}", s, v.decimal);
        }
    }

    #[test]
    fn test_parse() {
        expect_parse(r#"{ "decimal": "123.456" }"#, rust_decimal::Decimal::new(123456, 3));

        expect_parse(r#"{ "decimal": "444" }"#, rust_decimal::Decimal::new(444, 0));

        expect_parse(r#"{ "decimal": "444.0" }"#, rust_decimal::Decimal::new(444, 0));
        expect_parse(r#"{ "decimal": "444." }"#, rust_decimal::Decimal::new(444, 0));
        expect_parse(r#"{ "decimal": ".444" }"#, dec!(0.444));

        // expect_parse_error(r#"{ "decimal": "444." }"#);
        // expect_parse_error(r#"{ "decimal": ".444" }"#);

        expect_parse(r#"{ "decimal": "0.444" }"#, dec!(0.444));

        expect_parse(r#"{ "decimal": "0000.444" }"#, dec!(0.444));

        expect_parse(r#"{ "decimal": "876.444" }"#, dec!(876.444));

        expect_parse(r#"{ "decimal": "0E-18" }"#, dec!(0));
        expect_parse(r#"{ "decimal": "12345E-3" }"#, dec!(12.345));
        expect_parse(r#"{ "decimal": "-2.5e-2" }"#, dec!(-0.025));
        expect_parse(r#"{ "decimal": "1.5E+3" }"#, dec!(1500));
        expect_parse_error(r#"{ "decimal": "1.5E" }"#);

        expect_parse_error(r#"{ "decimal": "0.1.2" }"#);
    }

    fn expect_parse_as(s: &str, expected: &str) {
        let value: MyStruct = serde_json::from_str(s).unwrap();
        assert_eq!(value.decimal.to_string(), expected, "parsing {}", s);
    }

    #[test]
    fn trailing_zeros_after_the_decimal_point_are_removed() {
        expect_parse_as(r#"{ "decimal": "1.500" }"#, "1.5");
        expect_parse_as(r#"{ "decimal": "444.0" }"#, "444");
        expect_parse_as(r#"{ "decimal": "0.0000" }"#, "0");
        expect_parse_as(r#"{ "decimal": "0E-18" }"#, "0");
        expect_parse_as(r#"{ "decimal": "-0.00" }"#, "0");
        expect_parse_as(r#"{ "decimal": "100" }"#, "100");
        expect_parse_as(r#"{ "decimal": "28.8273" }"#, "28.8273");
    }

    #[test]
    fn test_serialize() {
        assert_eq!(
            serde_json::to_string(&MyStruct {
                decimal: Decimal::new(314159, 5)
            })
            .unwrap(),
            "{\"decimal\":\"3.14159\"}"
        );
    }

    #[test]
    fn equal_values_with_different_scales() {
        assert_eq!(Decimal::new(1, 1), Decimal::new(10, 2));
        assert_eq!(Decimal::new(100, 2), Decimal::new(1, 0));
    }
}
