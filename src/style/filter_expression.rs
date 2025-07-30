use bstr::{BStr, BString};
use serde::Deserialize;

use crate::{FeatureView, Value};

#[derive(Debug, Clone)]
pub enum FilterExpression {
    All(Vec<FilterExpression>),
    Any(Vec<FilterExpression>),
    In(BString, Vec<FilterValue>),
    NotIn(BString, Vec<FilterValue>),
    Has(FilterValue),
    NotHas(FilterValue),
    Cmp(BString, Comparison, FilterValue),
    True,
}

impl FilterExpression {
    pub fn eval(&self, feature: &FeatureView<'_>) -> bool {
        match self {
            FilterExpression::All(filters) => filters.iter().all(|f| f.eval(feature).into()).into(),
            FilterExpression::Any(filters) => filters.iter().any(|f| f.eval(feature).into()).into(),
            FilterExpression::In(tag, values) => {
                if let Some(value) = feature.key(tag) {
                    values.iter().any(|v| v == &value)
                } else {
                    false
                }
            }
            FilterExpression::NotIn(tag, values) => {
                if let Some(value) = feature.key(tag) {
                    values.iter().all(|v| v != &value)
                } else {
                    true
                }
            }
            FilterExpression::Has(tag) => tag.as_str().and_then(|k| feature.key(k)).is_some(),
            FilterExpression::NotHas(tag) => tag.as_str().and_then(|k| feature.key(k)).is_none(),
            FilterExpression::Cmp(tag, cmp, value) => feature
                .key(tag)
                .map(|v| cmp.cmp(value, &v))
                .unwrap_or(cmp.default()),
            FilterExpression::True => true,
        }
    }
}

impl Default for FilterExpression {
    fn default() -> Self {
        FilterExpression::True
    }
}

impl<'de> serde::de::Deserialize<'de> for FilterExpression {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(FilterVisitor)
    }
}

struct FilterVisitor;

impl<'de> serde::de::Visitor<'de> for FilterVisitor {
    type Value = FilterExpression;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "a filter array expression")
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
                FilterExpression::All(filters)
            }
            "any" => {
                let mut filters = Vec::new();
                while let Some(filter) = seq.next_element()? {
                    filters.push(filter)
                }
                FilterExpression::Any(filters)
            }
            "in" | "!in" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for in filter expression"))?;
                let mut values = Vec::new();
                while let Some(value) = seq.next_element()? {
                    values.push(value)
                }

                if kind == "in" {
                    FilterExpression::In(tag, values)
                } else {
                    FilterExpression::NotIn(tag, values)
                }
            }
            "has" | "!has" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for has filter expression"))?;

                if kind == "has" {
                    FilterExpression::Has(tag)
                } else {
                    FilterExpression::NotHas(tag)
                }
            }
            s @ _ => {
                if let Some(cmp) = Comparison::from_str(s) {
                    let tag = seq
                        .next_element()?
                        .ok_or(E::custom("expected tag for comparison filter expression"))?;
                    let value = seq
                        .next_element()?
                        .ok_or(E::custom("expected value for comparison filter expression"))?;

                    FilterExpression::Cmp(tag, cmp, value)
                } else {
                    return Err(E::custom(format!("unexpected filter type '{}'", kind)));
                }
            }
        };

        Ok(exp)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FilterValue {
    String(BString),
    Number(f64),
    Bool(bool),
}

impl FilterValue {
    fn as_str(&self) -> Option<&BStr> {
        match self {
            FilterValue::String(bstring) => Some(bstring.as_ref()),
            FilterValue::Number(_) => None,
            FilterValue::Bool(_) => None,
        }
    }
}

impl PartialEq<Value<'_>> for FilterValue {
    fn eq(&self, other: &Value<'_>) -> bool {
        match (self, other) {
            (FilterValue::String(s), Value::String(ss)) => s == *ss,
            (FilterValue::Number(n), Value::Number(nn)) => n == nn,
            (FilterValue::Bool(b), Value::Bool(bb)) => b == bb,
            _ => false,
        }
    }
}

impl PartialOrd<Value<'_>> for FilterValue {
    fn partial_cmp(&self, other: &Value<'_>) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (FilterValue::Number(n), Value::Number(nn)) => n.partial_cmp(nn),
            _ => None,
        }
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
    fn cmp(&self, l: &FilterValue, r: &Value<'_>) -> bool {
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

    fn default(&self) -> bool {
        match self {
            Comparison::Neq => true,
            _ => false,
        }
    }
}
