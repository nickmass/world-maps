#![allow(dead_code)]
use bstr::ByteSlice;
use serde::Deserialize;

use super::FeatureView;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Style {
    pub layers: Vec<Layer>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Layer {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: LayerType,
    #[serde(rename = "source-layer")]
    pub layer: Option<String>,
    pub minzoom: Option<f32>,
    pub maxzoom: Option<f32>,
    #[serde(flatten)]
    pub filter: Filter,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub paint: Paint,
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LayerType {
    Background,
    Fill,
    Line,
    Symbol,
    Raster,
    FillExtrusion,
}

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Layout {
    #[serde(default)]
    pub visibility: Visibility,
    #[serde(default)]
    pub line_cap: LineCap,
    #[serde(default)]
    pub line_join: LineJoin,
    pub text_allow_overlap: Option<bool>,
    text_anchor: Option<Parameter<TextAnchor>>,
    text_field: Option<String>,
    #[serde(default)]
    pub text_font: Vec<String>,
    pub text_ignore_placement: Option<bool>,
    pub text_letter_spacing: Option<f32>,
    pub text_max_width: Option<f32>,
    pub text_offset: Option<(f32, f32)>,
    pub text_optional: Option<bool>,
    pub text_padding: Option<f32>,
    pub text_rotation_alignment: Option<TextRotationAlignment>,
    text_size: Option<Parameter<f32>>,
    pub text_transform: Option<TextTransform>,
    symbol_placement: Option<Parameter<SymbolPlacement>>,
    pub symbol_spacing: Option<f32>,
}

impl Layout {
    pub fn text_size(&self, zoom: f32) -> f32 {
        get_parameter(self.text_size.as_ref(), zoom).unwrap_or(16.0)
    }

    pub fn symbol_placement(&self, zoom: f32) -> Option<SymbolPlacement> {
        get_parameter(self.symbol_placement.as_ref(), zoom)
    }

    pub fn text_max_width(&self) -> f32 {
        self.text_max_width.unwrap_or(10.0)
    }

    pub fn text(&self, view: &FeatureView<'_>) -> Option<smartstring::alias::String> {
        let format = self.text_field.as_ref()?;

        let mut text = smartstring::alias::String::new();
        let mut in_field = false;
        let mut span_start = 0;
        let transform = self.text_transform.unwrap_or_default();
        for (idx, c) in format.chars().enumerate() {
            match c {
                '{' if in_field == false => {
                    span_start = idx + 1;
                    in_field = true;
                }
                '}' if in_field == true => {
                    let field = &format[span_start..idx];
                    for c in view
                        .key(field)
                        .and_then(|v| v.as_str())
                        .iter()
                        .map(|s| s.chars())
                        .flatten()
                    {
                        transform.transform(c, &mut text);
                    }
                    in_field = false
                }
                c if in_field == false => {
                    transform.transform(c, &mut text);
                }
                _ => (),
            }
        }

        if text.trim().is_empty() {
            None
        } else {
            let text = text.trim().into();
            Some(text)
        }
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Visibility {
    Visible,
    None,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility::Visible
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LineJoin {
    Round,
    Miter,
    Bevel,
}

impl Default for LineJoin {
    fn default() -> Self {
        LineJoin::Miter
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

impl Default for LineCap {
    fn default() -> Self {
        LineCap::Butt
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
}

impl Default for TextTransform {
    fn default() -> Self {
        Self::None
    }
}

impl TextTransform {
    fn transform(&self, c: char, text: &mut smartstring::alias::String) {
        match self {
            TextTransform::None => text.push(c),
            TextTransform::Uppercase => text.extend(c.to_uppercase()),
            TextTransform::Lowercase => text.extend(c.to_lowercase()),
        }
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextAnchor {
    Center,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Default for TextAnchor {
    fn default() -> Self {
        TextAnchor::Center
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TextRotationAlignment {
    Auto,
    Map,
    Viewport,
}

impl Default for TextRotationAlignment {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolPlacement {
    Point,
    Line,
    LineCenter,
}

impl Default for SymbolPlacement {
    fn default() -> Self {
        Self::Point
    }
}

impl EnumParameter for SymbolPlacement {}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
pub struct Paint {
    background_color: Option<Parameter<Color>>,
    line_color: Option<Parameter<Color>>,
    line_opacity: Option<Parameter<f32>>,
    line_width: Option<Parameter<f32>>,
    line_dasharray: Option<Exists>,
    fill_antialias: Option<Parameter<bool>>,
    fill_color: Option<Parameter<Color>>,
    fill_opacity: Option<Parameter<f32>>,
    fill_outline_color: Option<Parameter<Color>>,
    fill_translate: Option<Parameter<(f32, f32)>>,
    fill_pattern: Option<Exists>,
    text_color: Option<Parameter<Color>>,
    text_halo_blur: Option<Parameter<f32>>,
    text_halo_color: Option<Parameter<Color>>,
    text_halo_width: Option<Parameter<f32>>,
}

impl Paint {
    pub fn background_color(&self, zoom: f32) -> Color {
        get_parameter(self.background_color.as_ref(), zoom).unwrap_or_default()
    }

    pub fn fill_color(&self, zoom: f32) -> Color {
        let color = get_parameter(self.fill_color.as_ref(), zoom);
        let opacity = get_parameter(self.fill_opacity.as_ref(), zoom);

        if let Some(color) = color {
            color.with_alpha(opacity.unwrap_or(color.alpha()))
        } else {
            Color::default()
        }
    }

    pub fn fill_outline_color(&self, zoom: f32) -> Option<Color> {
        let color = get_parameter(self.fill_outline_color.as_ref(), zoom);
        let opacity = get_parameter(self.fill_opacity.as_ref(), zoom);

        if let Some(color) = color {
            let color = color.with_alpha(opacity.unwrap_or(color.alpha()));

            Some(color)
        } else {
            None
        }
    }

    pub fn fill_translate(&self, zoom: f32) -> (f32, f32) {
        get_parameter(self.fill_translate.as_ref(), zoom).unwrap_or((0.0, 0.0))
    }

    pub fn line_color(&self, zoom: f32) -> Color {
        let color = get_parameter(self.line_color.as_ref(), zoom);
        let opacity = get_parameter(self.line_opacity.as_ref(), zoom);

        if let Some(color) = color {
            color.with_alpha(opacity.unwrap_or(color.alpha()))
        } else {
            Color::default()
        }
    }

    pub fn text_color(&self, zoom: f32) -> Color {
        get_parameter(self.text_color.as_ref(), zoom).unwrap_or_default()
    }

    pub fn text_halo_color(&self, zoom: f32) -> Color {
        get_parameter(self.text_halo_color.as_ref(), zoom).unwrap_or_default()
    }

    pub fn text_halo_width(&self, zoom: f32) -> f32 {
        get_parameter(self.text_halo_width.as_ref(), zoom).unwrap_or_default()
    }

    pub fn line_width(&self, zoom: f32) -> f32 {
        get_parameter(self.line_width.as_ref(), zoom).unwrap_or(1.0)
    }

    pub fn unsupported(&self) -> bool {
        self.fill_pattern.is_some()
    }
}

fn get_parameter<T: Copy + Interoplate>(param: Option<&'_ Parameter<T>>, zoom: f32) -> Option<T> {
    if let Some(param) = param {
        Some(param.eval(zoom))
    } else {
        None
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Color {
    Rgba(Rgba),
    Hsla(Hsla),
}

impl Color {
    pub fn to_rgba(&self) -> Rgba {
        let c = match self {
            Color::Rgba(c) => *c,
            Color::Hsla(c) => c.to_rgba(),
        };

        c
    }

    fn with_alpha(&self, alpha: f32) -> Color {
        let mut color = *self;
        match color {
            Color::Rgba(ref mut c) => c.a = alpha,
            Color::Hsla(ref mut c) => c.a = alpha,
        }

        color
    }

    fn alpha(&self) -> f32 {
        match self {
            Color::Rgba(c) => c.a,
            Color::Hsla(c) => c.a,
        }
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::Rgba(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })
    }
}

impl Interoplate for Color {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        match (self, other) {
            (Color::Rgba(last), Color::Rgba(next)) => Color::Rgba(last.interpolate(factor, next)),
            (Color::Hsla(last), Color::Hsla(next)) => Color::Hsla(last.interpolate(factor, next)),
            (Color::Rgba(last), Color::Hsla(next)) => {
                Color::Rgba(last.interpolate(factor, next.to_rgba()))
            }
            (Color::Hsla(last), Color::Rgba(next)) => {
                Color::Rgba(last.to_rgba().interpolate(factor, next))
            }
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Interoplate for Rgba {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        Rgba {
            r: self.r.interpolate(factor, other.r),
            g: self.g.interpolate(factor, other.g),
            b: self.b.interpolate(factor, other.b),
            a: self.a.interpolate(factor, other.a),
        }
    }
}

impl Interoplate for Hsla {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        Hsla {
            h: self.h.interpolate(factor, other.h),
            s: self.s.interpolate(factor, other.s),
            l: self.l.interpolate(factor, other.l),
            a: self.a.interpolate(factor, other.a),
        }
    }
}

impl Hsla {
    fn to_rgba(&self) -> Rgba {
        let h = self.h.min(1.0).max(0.0) * 360.0;
        let s = self.s.min(1.0).max(0.0);
        let l = self.l.min(1.0).max(0.0);

        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h_prime = h / 60.0;
        let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
        let (r, g, b) = match h_prime {
            v if 0.0 <= v && v <= 1.0 => (c, x, 0.0),
            v if 1.0 <= v && v <= 2.0 => (x, c, 0.0),
            v if 2.0 <= v && v <= 3.0 => (0.0, c, x),
            v if 3.0 <= v && v <= 4.0 => (0.0, x, c),
            v if 4.0 <= v && v <= 5.0 => (x, 0.0, c),
            v if 5.0 <= v && v <= 6.0 => (c, 0.0, x),
            _ => (0.0, 0.0, 0.0),
        };

        let m = l - (c / 2.0);

        let r = r + m;
        let g = g + m;
        let b = b + m;

        let a = self.a;

        Rgba { r, g, b, a }
    }
}

impl Interoplate for f32 {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        (factor * other) + ((1.0 - factor) * self)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let s = String::deserialize(deserializer)?;
        let to_b = |c| {
            if c >= b'0' && c <= b'9' {
                (c - b'0') as u8
            } else if c >= b'a' && c <= b'f' {
                ((c - b'a') as u8) + 10
            } else if c >= b'A' && c <= b'F' {
                ((c - b'A') as u8) + 10
            } else {
                0
            }
        };

        let color = if s.starts_with('#') && s.len() == 4 {
            let mut c = s.bytes();
            let _ = c.next();
            let r = c.next().map(to_b).unwrap_or_default() << 4;
            let g = c.next().map(to_b).unwrap_or_default() << 4;
            let b = c.next().map(to_b).unwrap_or_default() << 4;

            Color::Rgba(Rgba {
                r: r as f32 / 0xff as f32,
                g: g as f32 / 0xff as f32,
                b: b as f32 / 0xff as f32,
                a: 1.0,
            })
        } else if s.starts_with('#') && s.len() == 7 {
            let mut c = s.bytes();
            let _ = c.next();
            let r = c.next().map(to_b).unwrap_or_default() << 4;
            let rr = c.next().map(to_b).unwrap_or_default();
            let g = c.next().map(to_b).unwrap_or_default() << 4;
            let gg = c.next().map(to_b).unwrap_or_default();
            let b = c.next().map(to_b).unwrap_or_default() << 4;
            let bb = c.next().map(to_b).unwrap_or_default();

            let r = r | rr;
            let g = g | gg;
            let b = b | bb;

            Color::Rgba(Rgba {
                r: r as f32 / 0xff as f32,
                g: g as f32 / 0xff as f32,
                b: b as f32 / 0xff as f32,
                a: 1.0,
            })
        } else if s.starts_with("rgba(") {
            let (_, n) = s
                .split_once("rgba(")
                .ok_or(D::Error::custom("invalid rgba color"))?;
            let (n, _) = n
                .split_once(")")
                .ok_or(D::Error::custom("invalid rgba color"))?;

            let mut parts = n.split(',');
            let r = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let g = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let b = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let a = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Rgba(Rgba {
                r: r / 255.0,
                g: g / 255.0,
                b: b / 255.0,
                a,
            })
        } else if s.starts_with("rgb(") {
            let (_, n) = s
                .split_once("rgb(")
                .ok_or(D::Error::custom("invalid rgb color"))?;
            let (n, _) = n
                .split_once(")")
                .ok_or(D::Error::custom("invalid rgb color"))?;

            let mut parts = n.split(',');
            let r = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let g = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let b = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Rgba(Rgba {
                r: r / 255.0,
                g: g / 255.0,
                b: b / 255.0,
                a: 1.0,
            })
        } else if s.starts_with("hsla(") {
            let (_, n) = s
                .split_once("hsla(")
                .ok_or(D::Error::custom("invalid hsla color"))?;
            let (n, _) = n
                .split_once(")")
                .ok_or(D::Error::custom("invalid hsla color"))?;

            let mut parts = n.split(',');
            let h = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let s = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let l = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let a = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Hsla(Hsla {
                h: h / 360.0,
                s: s / 100.0,
                l: l / 100.0,
                a,
            })
        } else if s.starts_with("hsl(") {
            let (_, n) = s
                .split_once("hsl(")
                .ok_or(D::Error::custom("invalid hsl color"))?;
            let (n, _) = n
                .split_once(")")
                .ok_or(D::Error::custom("invalid hsl color"))?;

            let mut parts = n.split(',');
            let h = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let s = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let l = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Hsla(Hsla {
                h: h / 360.0,
                s: s / 100.0,
                l: l / 100.0,
                a: 1.0,
            })
        } else {
            return Err(D::Error::custom("invalid color"));
        };

        Ok(color)
    }
}

fn str_to_f32<I: IntoIterator<Item = u8>>(iter: I) -> f32 {
    let mut whole = 0.0;
    let mut frac = None;
    let mut frac_scale = 1.0;

    for b in iter {
        let n = if b >= b'0' && b <= b'9' {
            (b - b'0') as f32
        } else if b == b'.' {
            frac = Some(0.0);
            continue;
        } else {
            continue;
        };

        if let Some(frac) = frac.as_mut() {
            frac_scale *= 0.1;
            *frac += frac_scale * n;
        } else {
            whole = (whole * 10.0) + n;
        }
    }

    whole + frac.unwrap_or(0.0)
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum Parameter<T> {
    Constant(T),
    Function(Function<T>),
}

impl<T: Copy + Interoplate> Parameter<T> {
    fn eval(&self, zoom: f32) -> T {
        match self {
            Parameter::Constant(v) => *v,
            Parameter::Function(int) => int.eval(zoom),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
struct Function<T> {
    base: Option<f32>,
    stops: smallvec::SmallVec<[(f32, T); 8]>,
}

impl<T: Copy + Interoplate> Function<T> {
    fn eval(&self, zoom: f32) -> T {
        assert!(self.stops.len() != 0);

        let first = self.stops.first().unwrap();
        let last = self.stops.last().unwrap();

        if zoom <= first.0 || self.stops.len() == 1 {
            first.1
        } else if zoom >= last.0 {
            last.1
        } else {
            let mut steps = self.stops.iter();
            let last = steps.next().unwrap();

            for next in steps {
                if zoom >= last.0 && zoom <= next.0 {
                    let range = next.0 - last.0;
                    let start = zoom - last.0;

                    let n = start / range;
                    return last.1.interpolate(n.powf(self.base.unwrap_or(1.0)), next.1);
                }
            }

            unreachable!()
        }
    }
}

trait Interoplate {
    fn interpolate(&self, factor: f32, other: Self) -> Self;
}

impl<A: Interoplate, B: Interoplate> Interoplate for (A, B) {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        (
            self.0.interpolate(factor, other.0),
            self.1.interpolate(factor, other.1),
        )
    }
}

trait EnumParameter: Copy {}

impl<T: EnumParameter> Interoplate for T {
    fn interpolate(&self, _factor: f32, _other: Self) -> Self {
        *self
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Exists;

impl<'de> serde::Deserialize<'de> for Exists {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let _ = serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Exists)
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Filter {
    #[serde(default)]
    filter: FilterExpression,
}

impl Filter {
    pub fn eval(&self, feature: &FeatureView<'_>) -> bool {
        self.filter.eval(feature)
    }
}

#[derive(Debug, Clone)]
enum FilterExpression {
    All(Vec<FilterExpression>),
    Any(Vec<FilterExpression>),
    In(String, Vec<FilterValue>),
    NotIn(String, Vec<FilterValue>),
    Has(String),
    NotHas(String),
    Eq(String, FilterValue),
    Neq(String, FilterValue),
    Lteq(String, FilterValue),
    Gteq(String, FilterValue),
    Lt(String, FilterValue),
    Gt(String, FilterValue),
    True,
}

impl FilterExpression {
    pub fn eval(&self, feature: &FeatureView<'_>) -> bool {
        match self {
            FilterExpression::All(filters) => filters.iter().all(|f| f.eval(feature)),
            FilterExpression::Any(filters) => filters.iter().any(|f| f.eval(feature)),
            FilterExpression::In(tag, values) => {
                if let Some(value) = feature.key(&tag) {
                    values.iter().any(|v| v == value)
                } else {
                    false
                }
            }
            FilterExpression::NotIn(tag, values) => {
                if let Some(value) = feature.key(&tag) {
                    values.iter().all(|v| v != value)
                } else {
                    true
                }
            }
            FilterExpression::Has(tag) => feature.key(&tag).is_some(),
            FilterExpression::NotHas(tag) => feature.key(&tag).is_none(),
            FilterExpression::Eq(tag, value) => {
                feature.key(&tag).map(|v| value == v).unwrap_or(false)
            }
            FilterExpression::Neq(tag, value) => {
                feature.key(&tag).map(|v| value != v).unwrap_or(true)
            }
            FilterExpression::Lteq(tag, value) => {
                feature.key(&tag).map(|v| value > v).unwrap_or(false)
            }
            FilterExpression::Gteq(tag, value) => {
                feature.key(&tag).map(|v| value < v).unwrap_or(false)
            }
            FilterExpression::Lt(tag, value) => {
                feature.key(&tag).map(|v| value >= v).unwrap_or(false)
            }
            FilterExpression::Gt(tag, value) => {
                feature.key(&tag).map(|v| value <= v).unwrap_or(false)
            }
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
        deserializer.deserialize_seq(FilterVisitor::new())
    }
}

struct FilterVisitor;

impl FilterVisitor {
    fn new() -> Self {
        FilterVisitor
    }
}

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
            "==" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for == filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for == filter expression"))?;

                FilterExpression::Eq(tag, value)
            }
            "!=" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for != filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for != filter expression"))?;

                FilterExpression::Neq(tag, value)
            }
            "<=" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for <= filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for <= filter expression"))?;

                FilterExpression::Lteq(tag, value)
            }
            ">=" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for >= filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for >= filter expression"))?;

                FilterExpression::Gteq(tag, value)
            }
            "<" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for < filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for < filter expression"))?;

                FilterExpression::Lt(tag, value)
            }
            ">" => {
                let tag = seq
                    .next_element()?
                    .ok_or(E::custom("expected tag for > filter expression"))?;
                let value = seq
                    .next_element()?
                    .ok_or(E::custom("expected value for > filter expression"))?;

                FilterExpression::Gt(tag, value)
            }
            _ => return Err(E::custom(format!("unexpected filter type '{}'", kind))),
        };

        Ok(exp)
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum FilterValue {
    String(String),
    Number(f64),
}

impl PartialEq<super::Value<'_>> for &FilterValue {
    fn eq(&self, other: &super::Value<'_>) -> bool {
        match (self, other) {
            (FilterValue::String(s), super::Value::String(ss)) => s == ss,
            (FilterValue::Number(n), super::Value::Number(nn)) => n == nn,
            _ => false,
        }
    }
}

impl PartialOrd<super::Value<'_>> for &FilterValue {
    fn partial_cmp(&self, other: &super::Value<'_>) -> Option<std::cmp::Ordering> {
        match (self, other) {
            (FilterValue::Number(n), super::Value::Number(nn)) => n.partial_cmp(nn),
            _ => None,
        }
    }
}
