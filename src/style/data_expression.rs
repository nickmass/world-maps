use bstr::{BStr, BString, ByteSlice};

use super::{Color, Parameter};
use crate::{FeatureView, Value};

#[derive(Debug, Clone)]
pub enum DataExpression<'a> {
    All(Vec<DataExpression<'a>>),
    Any(Vec<DataExpression<'a>>),
    In(Box<DataExpression<'a>>, Vec<DataExpression<'a>>),
    Has(Box<DataExpression<'a>>),
    Get(Box<DataExpression<'a>>),
    Cmp(Comparison, Box<DataExpression<'a>>, Box<DataExpression<'a>>),
    ToBoolean(Box<DataExpression<'a>>),
    Match(
        Box<DataExpression<'a>>,
        Vec<(DataExpression<'a>, DataExpression<'a>)>,
        Box<DataExpression<'a>>,
    ),
    Case(
        Vec<(DataExpression<'a>, DataExpression<'a>)>,
        Box<DataExpression<'a>>,
    ),
    Constant(ExpressionValue<'a>),
}

impl<'a> DataExpression<'a> {
    pub fn eval<'f>(&'a self, feature: &'f FeatureView<'_>) -> ExpressionValue<'f>
    where
        'a: 'f,
    {
        match self {
            DataExpression::All(filters) => filters.iter().all(|f| f.eval(feature).into()).into(),
            DataExpression::Any(filters) => filters.iter().any(|f| f.eval(feature).into()).into(),
            DataExpression::In(needle, values) => {
                let needle = needle.eval(feature);
                values.iter().any(|v| v.eval(feature) == needle).into()
            }
            DataExpression::Has(tag) => tag
                .eval(feature)
                .as_str()
                .and_then(|k| feature.key(k))
                .is_some()
                .into(),
            DataExpression::Cmp(cmp, l, r) => cmp.cmp(&l.eval(feature), &r.eval(feature)).into(),
            DataExpression::ToBoolean(value) => bool::from(value.eval(feature)).into(),
            DataExpression::Match(input, cases, fallback) => {
                let input = input.eval(feature);
                for (label, value) in cases {
                    let label = label.eval(feature);
                    if input == label {
                        return value.eval(feature);
                    }
                }

                fallback.eval(feature)
            }
            DataExpression::Case(cases, fallback) => {
                for (condition, value) in cases {
                    let condition = bool::from(condition.eval(feature));
                    if condition {
                        return value.eval(feature);
                    }
                }

                fallback.eval(feature)
            }
            DataExpression::Get(value) => {
                let tag = value.eval(feature).as_str().and_then(|v| feature.key(v));
                match tag {
                    Some(s) => s.into(),
                    None => ExpressionValue::Null,
                }
            }
            DataExpression::Constant(value) => value.ref_clone(),
        }
    }

    pub fn is_computed_from_feature(&self) -> bool {
        let child = |exp: &DataExpression| exp.is_computed_from_feature();
        let children = |exps: &[DataExpression]| exps.iter().any(child);
        match self {
            DataExpression::All(exps) => children(exps),
            DataExpression::Any(exps) => children(exps),
            DataExpression::ToBoolean(exp) => child(exp),
            DataExpression::Case(cases, fallback) => {
                child(fallback) || cases.iter().any(|(l, v)| child(l) || child(v))
            }
            DataExpression::Match(input, cases, fallback) => {
                child(input) || child(fallback) || cases.iter().any(|(l, v)| child(l) || child(v))
            }
            DataExpression::Constant(_) => false,
            DataExpression::Get(_) => true,
            DataExpression::Has(_) => true,
            DataExpression::In(needle, values) => child(needle) || children(values),
            DataExpression::Cmp(_, left, right) => child(left) || child(right),
        }
    }
}

impl<'a> Default for DataExpression<'a> {
    fn default() -> Self {
        DataExpression::Constant(ExpressionValue::Bool(true))
    }
}

impl<'de> serde::de::Deserialize<'de> for DataExpression<'static> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ExpressionVisitor)
    }
}

struct ExpressionVisitor;

macro_rules! visit_ty {
    ($fn:ident($ty:ident)) => {
        fn $fn<E>(self, v: $ty) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(DataExpression::Constant(v.into()))
        }
    };
}

impl<'de> serde::de::Visitor<'de> for ExpressionVisitor {
    type Value = DataExpression<'static>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "a constant or filter array expression")
    }

    visit_ty!(visit_bool(bool));
    visit_ty!(visit_string(String));
    visit_ty!(visit_f32(f32));
    visit_ty!(visit_f64(f64));
    visit_ty!(visit_i8(i8));
    visit_ty!(visit_i16(i16));
    visit_ty!(visit_i32(i32));
    visit_ty!(visit_u8(u8));
    visit_ty!(visit_u16(u16));
    visit_ty!(visit_u32(u32));

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DataExpression::Constant(v.to_string().into()))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DataExpression::Constant((v as f64).into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(DataExpression::Constant((v as f64).into()))
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        use serde::de::Error as E;

        let kind: String = seq
            .next_element()?
            .ok_or(E::custom("expected filter expression type"))?;

        let exp = match kind.as_str() {
            "all" => {
                let mut filters = Vec::new();
                while let Some(filter) = seq.next_element()? {
                    filters.push(filter)
                }
                DataExpression::All(filters)
            }
            "any" => {
                let mut filters = Vec::new();
                while let Some(filter) = seq.next_element()? {
                    filters.push(filter)
                }
                DataExpression::Any(filters)
            }
            "in" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for in filter expression"))?;
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value)
                }

                DataExpression::In(tag, values)
            }
            "has" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for has filter expression"))?;

                DataExpression::Has(tag)
            }
            "to-boolean" => {
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for to-boolean expression"))?;

                DataExpression::ToBoolean(value)
            }
            "get" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for > filter expression"))?;

                DataExpression::Get(tag)
            }
            "case" => {
                let mut cases = Vec::new();
                let fallback = loop {
                    let condition = seq.next_element()?;
                    let value = seq.next_element()?;

                    match (condition, value) {
                        (Some(condition), Some(value)) => {
                            cases.push((condition, value));
                        }
                        (Some(fallback), None) => {
                            break fallback;
                        }
                        (None, Some(_)) => {
                            return Err(E::custom(
                                "expected label for each case in case expression",
                            ));
                        }
                        (None, None) => {
                            return Err(E::custom("expected fallback case for case expression"));
                        }
                    }
                };

                DataExpression::Case(cases, Box::new(fallback))
            }
            "match" => {
                let input = seq
                    .next_element()?
                    .ok_or(E::custom("expected input for match expression"))?;

                let mut cases = Vec::new();
                let fallback = loop {
                    let label = seq.next_element()?;
                    let value = seq.next_element()?;

                    match (label, value) {
                        (Some(label @ DataExpression::Constant(_)), Some(value)) => {
                            cases.push((label, value));
                        }
                        (Some(_), Some(_)) => {
                            return Err(E::custom("expected constant label for match expression"));
                        }
                        (Some(fallback), None) => {
                            break fallback;
                        }
                        (None, Some(_)) => {
                            return Err(E::custom(
                                "expected label for each case in match expression",
                            ));
                        }
                        (None, None) => {
                            return Err(E::custom("expected fallback case for match expression"));
                        }
                    }
                };

                DataExpression::Match(input, cases, Box::new(fallback))
            }
            s @ _ => {
                if let Some(cmp) = Comparison::from_str(s) {
                    let left = seq
                        .next_element()?
                        .ok_or(E::custom("expected left for comparison expression"))?;
                    let right = seq
                        .next_element()?
                        .ok_or(E::custom("expected right value for comparison expression"))?;
                    DataExpression::Cmp(cmp, left, right)
                } else {
                    return Err(E::custom(format!("unexpected filter type '{}'", kind)));
                }
            }
        };

        Ok(exp)
    }
}

#[derive(Debug, Clone)]
pub enum ExpressionValue<'a> {
    String(BString),
    Str(&'a BStr),
    Number(f64),
    Bool(bool),
    Color(Color),
    Null,
}

impl<'a> ExpressionValue<'a> {
    pub fn as_str(&self) -> Option<&'_ BStr> {
        match self {
            ExpressionValue::String(s) => Some(s.as_ref()),
            ExpressionValue::Str(s) => Some(s),
            ExpressionValue::Number(_) => None,
            ExpressionValue::Bool(_) => None,
            ExpressionValue::Color(_) => None,
            ExpressionValue::Null => None,
        }
    }

    fn ref_clone(&self) -> ExpressionValue<'_> {
        match self {
            ExpressionValue::String(s) => ExpressionValue::Str(s.as_ref()),
            _ => self.clone(),
        }
    }

    pub fn to_parameter<O: Default + TryFrom<Self>>(self) -> Parameter<O> {
        let value = O::try_from(self).ok();
        Parameter::Constant(value)
    }
}

macro_rules! filter_value_from {
    ($($name:ident($ty:ident)),*) => {
        $(
            impl<'a> From<$ty> for ExpressionValue<'a> {
                fn from(value: $ty) -> Self {
                    ExpressionValue::$name(value.into())
                }
            }
        )*
    }
}

filter_value_from! {Bool(bool), String(String), Number(f32), Number(f64), Number(i32), Number(i16), Number(i8), Number(u32), Number(u16), Number(u8)}

impl<'a> From<ExpressionValue<'a>> for bool {
    fn from(value: ExpressionValue) -> Self {
        (&value).into()
    }
}

impl<'a> From<&'_ ExpressionValue<'a>> for bool {
    fn from(value: &ExpressionValue) -> Self {
        match value {
            ExpressionValue::String(s) => !s.is_empty(),
            ExpressionValue::Str(s) => !s.is_empty(),
            ExpressionValue::Number(n) => *n != 0.0 && !n.is_nan(),
            ExpressionValue::Bool(b) => *b,
            ExpressionValue::Color(_) => false,
            ExpressionValue::Null => false,
        }
    }
}

impl<'a> From<Value<'a>> for ExpressionValue<'a> {
    fn from(value: Value<'a>) -> Self {
        match value {
            Value::String(bstr) => ExpressionValue::Str(bstr),
            Value::Number(n) => ExpressionValue::Number(n),
            Value::Bool(b) => ExpressionValue::Bool(b),
        }
    }
}

impl<'a> PartialEq for ExpressionValue<'a> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExpressionValue::String(l), ExpressionValue::String(r)) => l == r,
            (ExpressionValue::String(l), ExpressionValue::Str(r)) => l == r,
            (ExpressionValue::Str(l), ExpressionValue::String(r)) => l == r,
            (ExpressionValue::Str(l), ExpressionValue::Str(r)) => l == r,
            (ExpressionValue::Number(l), ExpressionValue::Number(r)) => l == r,
            (ExpressionValue::Bool(l), ExpressionValue::Bool(r)) => l == r,
            (ExpressionValue::Color(l), ExpressionValue::Color(r)) => l == r,
            _ => false,
        }
    }
}

impl<'a> PartialOrd for ExpressionValue<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (Self::Number(n), Self::Number(nn)) => n.partial_cmp(nn),
            _ => None,
        }
    }
}

impl TryFrom<ExpressionValue<'_>> for f32 {
    type Error = ();

    fn try_from(value: ExpressionValue<'_>) -> Result<Self, Self::Error> {
        if let ExpressionValue::Number(v) = value {
            Ok(v as f32)
        } else {
            Err(())
        }
    }
}

impl TryFrom<ExpressionValue<'_>> for Color {
    type Error = ();

    fn try_from(value: ExpressionValue<'_>) -> Result<Self, Self::Error> {
        match value {
            ExpressionValue::String(bstring) => {
                bstring.to_str().map_err(|_| ())?.parse().map_err(|_| ())
            }
            ExpressionValue::Str(bstr) => bstr.to_str().map_err(|_| ())?.parse().map_err(|_| ()),
            ExpressionValue::Color(color) => Ok(color),
            _ => Err(()),
        }
    }
}

impl TryFrom<ExpressionValue<'_>> for (f32, f32) {
    type Error = ();

    fn try_from(_value: ExpressionValue<'_>) -> Result<Self, Self::Error> {
        panic!("data expressions for array values is not currently supported")
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Comparison {
    Eq,
    Neq,
    Lteq,
    GtEq,
    Lt,
    Gt,
}

impl Comparison {
    fn cmp(&self, l: &ExpressionValue<'_>, r: &ExpressionValue<'_>) -> bool {
        match self {
            Comparison::Eq => l == r,
            Comparison::Neq => l != r,
            Comparison::Lteq => l <= r,
            Comparison::GtEq => l >= r,
            Comparison::Lt => l < r,
            Comparison::Gt => l > r,
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        let v = match s {
            "==" => Comparison::Eq,
            "!=" => Comparison::Neq,
            "<=" => Comparison::Lteq,
            ">=" => Comparison::GtEq,
            "<" => Comparison::Lt,
            ">" => Comparison::Gt,
            _ => return None,
        };

        Some(v)
    }
}
