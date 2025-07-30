use bstr::{BStr, BString, ByteSlice};
use serde::Deserialize;
use smallvec::SmallVec;

use super::FeatureView;

use std::collections::HashMap;

pub mod color;
mod data_expression;
mod filter_expression;
mod source;

use color::*;
use data_expression::{DataExpression, ExpressionValue};
use filter_expression::FilterExpression;
pub use source::{Source, SourceCollection, SourceId};

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Style {
    pub sources: SourceCollection,
    pub layers: Vec<Layer>,
}

impl Style {
    pub fn load<R: std::io::Read>(reader: R) -> Result<Self, serde_json::Error> {
        let mut style: Self = serde_json::from_reader(reader)?;
        style.remap_source_ids();
        style.print_remaining_fields();

        Ok(style)
    }

    fn print_remaining_fields(&self) {
        use std::collections::HashSet;
        let mut layout_fields = HashSet::new();
        let mut paint_fields = HashSet::new();
        layout_fields.extend(
            self.layers
                .iter()
                .flat_map(|l| l.layout.remaining_fields.keys()),
        );
        paint_fields.extend(
            self.layers
                .iter()
                .flat_map(|l| l.paint.remaining_fields.keys()),
        );

        println!("Unsupported layout fields:");
        for field in layout_fields {
            println!("\t{field}");
        }
        println!();

        println!("Unsupported paint fields:");
        for field in paint_fields {
            println!("\t{field}");
        }
    }

    fn remap_source_ids(&mut self) {
        for layer in self.layers.iter_mut() {
            if let Some(id) = layer.source.take() {
                layer.source = self.sources.remap(id);
            }
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct Layer {
    pub id: String,
    pub source: Option<SourceId>,
    #[serde(rename = "type")]
    pub kind: LayerType,
    #[serde(rename = "source-layer")]
    pub layer: Option<String>,
    pub minzoom: Option<f32>,
    pub maxzoom: Option<f32>,
    #[serde(default)]
    filter: Filter,
    #[serde(default)]
    pub layout: Layout,
    #[serde(default)]
    pub paint: PaintFields,
}

impl Layer {
    pub fn filter(&self, features: &FeatureView<'_>) -> bool {
        self.filter.eval(features)
    }
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
#[serde(default)]
pub struct Layout {
    pub visibility: Visibility,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    // pub text_allow_overlap: Option<bool>,
    text_anchor: Field<TextAnchor>,
    text_field: Option<BString>,
    pub text_font: Vec<String>,
    // pub text_ignore_placement: Option<bool>,
    pub text_letter_spacing: Option<f32>,
    pub text_max_width: Option<f32>,
    pub text_offset: Option<(f32, f32)>,
    //pub text_optional: Option<bool>,
    pub text_padding: Option<f32>,
    pub text_rotation_alignment: Option<TextRotationAlignment>,
    text_size: Field<f32>,
    pub text_transform: Option<TextTransform>,
    symbol_placement: Field<SymbolPlacement>,
    //symbol_spacing: Option<f32>,
    #[serde(flatten)]
    remaining_fields: HashMap<String, Exists>,
}

impl Layout {
    pub fn text_size(&self, features: &FeatureView<'_>, zoom: f32) -> f32 {
        self.text_size.eval(features).eval(zoom).unwrap_or(16.0)
    }

    pub fn symbol_placement(
        &self,
        features: &FeatureView<'_>,
        zoom: f32,
    ) -> Option<SymbolPlacement> {
        self.symbol_placement.eval(features).eval(zoom)
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
                    let field = BStr::new(field);
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

impl TryFrom<ExpressionValue<'_>> for SymbolPlacement {
    type Error = ();

    fn try_from(value: ExpressionValue<'_>) -> Result<Self, Self::Error> {
        let value: Option<&[u8]> = value.as_str().map(|s| s.as_ref());
        match value {
            Some(b"point") => Ok(Self::Point),
            Some(b"line") => Ok(Self::Line),
            Some(b"line-center") => Ok(Self::LineCenter),
            _ => Err(()),
        }
    }
}

impl EnumParameter for SymbolPlacement {}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "kebab-case")]
#[serde(default)]
pub struct PaintFields {
    background_color: Field<Color>,
    line_color: Field<Color>,
    line_opacity: Field<f32>,
    line_width: Field<f32>,
    line_dasharray: Option<SmallVec<[f32; 8]>>,
    line_gap_width: Option<Exists>,
    fill_antialias: Field<bool>,
    fill_color: Field<Color>,
    fill_opacity: Field<f32>,
    fill_outline_color: Field<Color>,
    fill_translate: Field<(f32, f32)>,
    fill_pattern: Option<Exists>,
    text_color: Field<Color>,
    text_opacity: Field<f32>,
    text_halo_blur: Field<f32>,
    text_halo_color: Field<Color>,
    text_halo_width: Field<f32>,
    #[serde(flatten)]
    remaining_fields: HashMap<String, Exists>,
}

impl PaintFields {
    pub fn eval(&self, features: &FeatureView<'_>) -> Paint {
        Paint {
            background_color: self.background_color.eval(features),
            line_color: self.line_color.eval(features),
            line_opacity: self.line_opacity.eval(features),
            line_width: self.line_width.eval(features),
            line_dasharray: self.line_dasharray.clone(),
            fill_antialias: self.fill_antialias.eval(features),
            fill_color: self.fill_color.eval(features),
            fill_opacity: self.fill_opacity.eval(features),
            fill_outline_color: self.fill_outline_color.eval(features),
            fill_translate: self.fill_translate.eval(features),
            text_color: self.text_color.eval(features),
            text_opacity: self.text_opacity.eval(features),
            text_halo_blur: self.text_halo_blur.eval(features),
            text_halo_color: self.text_halo_color.eval(features),
            text_halo_width: self.text_halo_width.eval(features),
        }
    }

    pub fn unsupported(&self) -> bool {
        self.fill_pattern.is_some() || self.line_gap_width.is_some()
    }

    pub fn is_computed_from_feature(&self) -> bool {
        self.background_color.is_computer_from_feature()
            || self.line_color.is_computer_from_feature()
            || self.line_opacity.is_computer_from_feature()
            || self.line_width.is_computer_from_feature()
            || self.fill_antialias.is_computer_from_feature()
            || self.fill_color.is_computer_from_feature()
            || self.fill_opacity.is_computer_from_feature()
            || self.fill_outline_color.is_computer_from_feature()
            || self.fill_translate.is_computer_from_feature()
            || self.text_color.is_computer_from_feature()
            || self.text_opacity.is_computer_from_feature()
            || self.text_halo_blur.is_computer_from_feature()
            || self.text_halo_color.is_computer_from_feature()
            || self.text_halo_width.is_computer_from_feature()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paint {
    background_color: Parameter<Color>,
    line_color: Parameter<Color>,
    line_opacity: Parameter<f32>,
    line_width: Parameter<f32>,
    line_dasharray: Option<SmallVec<[f32; 8]>>,
    fill_antialias: Parameter<bool>,
    fill_color: Parameter<Color>,
    fill_opacity: Parameter<f32>,
    fill_outline_color: Parameter<Color>,
    fill_translate: Parameter<(f32, f32)>,
    text_color: Parameter<Color>,
    text_opacity: Parameter<f32>,
    text_halo_blur: Parameter<f32>,
    text_halo_color: Parameter<Color>,
    text_halo_width: Parameter<f32>,
}

impl Paint {
    pub fn background_color(&self, zoom: f32) -> Color {
        self.background_color.eval(zoom).unwrap_or_default()
    }

    pub fn fill_antialias(&self, zoom: f32) -> bool {
        self.fill_antialias.eval(zoom).unwrap_or(true)
    }

    pub fn fill_color(&self, zoom: f32) -> Color {
        let color = self.fill_color.eval(zoom);
        let opacity = self.fill_opacity.eval(zoom);

        if let Some(color) = color {
            color.with_alpha(opacity.unwrap_or(color.alpha()))
        } else {
            Color::default()
        }
    }

    pub fn fill_outline_color(&self, zoom: f32) -> Option<Color> {
        let color = self.fill_outline_color.eval(zoom);
        let opacity = self.fill_opacity.eval(zoom);

        if let Some(color) = color {
            let color = color.with_alpha(opacity.unwrap_or(color.alpha()));

            Some(color)
        } else {
            None
        }
    }

    pub fn fill_translate(&self, zoom: f32) -> (f32, f32) {
        self.fill_translate.eval(zoom).unwrap_or((0.0, 0.0))
    }

    pub fn line_color(&self, zoom: f32) -> Color {
        let color = self.line_color.eval(zoom);
        let opacity = self.line_opacity.eval(zoom);

        if let Some(color) = color {
            color.with_alpha(opacity.unwrap_or(color.alpha()))
        } else {
            Color::default()
        }
    }

    pub fn text_color(&self, zoom: f32) -> Color {
        let color = self.text_color.eval(zoom);
        let opacity = self.text_opacity.eval(zoom);

        if let Some(color) = color {
            color.with_alpha(opacity.unwrap_or(color.alpha()))
        } else {
            Color::default()
        }
    }

    pub fn text_halo_blur(&self, zoom: f32) -> f32 {
        self.text_halo_blur.eval(zoom).unwrap_or_default()
    }

    pub fn text_halo_color(&self, zoom: f32) -> Color {
        self.text_halo_color.eval(zoom).unwrap_or_default()
    }

    pub fn text_halo_width(&self, zoom: f32) -> f32 {
        self.text_halo_width.eval(zoom).unwrap_or_default()
    }

    pub fn line_width(&self, zoom: f32) -> f32 {
        self.line_width.eval(zoom).unwrap_or(1.0)
    }

    pub fn line_dasharray(&self) -> SmallVec<[f32; 8]> {
        self.line_dasharray.clone().unwrap_or(SmallVec::new())
    }
}

impl Interpolate for f32 {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        (factor * other) + ((1.0 - factor) * self)
    }
}

impl<T: Interpolate> Interpolate for Option<T> {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        if let Some(value) = self
            && let Some(other) = other
        {
            Some(value.interpolate(factor, other))
        } else {
            None
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
struct Function<T> {
    base: Option<f32>,
    property: Option<BString>,
    stops: smallvec::SmallVec<[(f32, T); 8]>,
}

impl<T> Function<T> {
    fn is_computed_from_feature(&self) -> bool {
        self.property.is_some()
    }
}

impl<T: Copy + Interpolate> Function<T> {
    fn eval(&self, feature: &FeatureView<'_>) -> Parameter<T> {
        let Some(property) = self.property.as_ref() else {
            return Parameter::ZoomFunction(ZoomFunction {
                base: self.base,
                stops: self.stops.clone(),
            });
        };

        let value = match feature.key(property) {
            Some(super::Value::Number(n)) => n,
            _ => 0.0,
        };

        let value = value as f32;

        assert!(self.stops.len() != 0);

        let first = self.stops.first().unwrap();
        let last = self.stops.last().unwrap();

        let result = if value <= first.0 || self.stops.len() == 1 {
            first.1
        } else if value >= last.0 {
            last.1
        } else {
            let mut steps = self.stops.iter();
            let mut last = steps.next().unwrap();

            let mut result = None;

            for next in steps {
                if value >= last.0 && value <= next.0 {
                    let range = next.0 - last.0;
                    let start = value - last.0;

                    let n = start / range;

                    let base = self.base.unwrap_or(1.0);

                    if base == 1.0 {
                        result = Some(last.1.interpolate(n.powf(self.base.unwrap_or(1.0)), next.1));
                        break;
                    } else {
                        let factor = (base.powf(start) - 1.0) / (base.powf(range) - 1.0);
                        result = Some(last.1.interpolate(factor, next.1));
                        break;
                    }
                }

                last = next;
            }

            result.unwrap()
        };

        Parameter::Constant(Some(result))
    }
}

pub trait Interpolate {
    fn interpolate(&self, factor: f32, other: Self) -> Self;
}

impl<A: Interpolate, B: Interpolate> Interpolate for (A, B) {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        (
            self.0.interpolate(factor, other.0),
            self.1.interpolate(factor, other.1),
        )
    }
}

trait EnumParameter: Copy {}

impl<T: EnumParameter> Interpolate for T {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        if factor < 0.5 { *self } else { other }
    }
}

impl Interpolate for bool {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        if factor < 0.5 { *self } else { other }
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

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum Filter {
    FilterExpression(FilterExpression),
    DataExpression(DataExpression<'static>),
}

impl Default for Filter {
    fn default() -> Self {
        Filter::FilterExpression(filter_expression::FilterExpression::True)
    }
}

impl Filter {
    pub fn eval(&self, feature: &FeatureView<'_>) -> bool {
        match self {
            Filter::FilterExpression(exp) => exp.eval(feature),
            Filter::DataExpression(exp) => exp.eval(feature).into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Field<O> {
    Constant(Option<O>),
    Function(Function<O>),
    DataExpression(DataExpression<'static>),
}

impl<O> Default for Field<O> {
    fn default() -> Self {
        Field::Constant(None)
    }
}

impl<O> Field<O> {
    fn is_computer_from_feature(&self) -> bool {
        match self {
            Field::Constant(_) => false,
            Field::Function(function) => function.is_computed_from_feature(),
            Field::DataExpression(exp) => exp.is_computed_from_feature(),
        }
    }
}

impl<'f, O: Copy + Interpolate + Default + TryFrom<ExpressionValue<'f>>> Field<O> {
    fn eval<'a: 'f>(&'a self, feature: &'f FeatureView<'_>) -> Parameter<O> {
        match self {
            Field::Constant(c) => Parameter::Constant(*c),
            Field::Function(f) => f.eval(feature),
            Field::DataExpression(exp) => exp.eval(feature).to_parameter(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parameter<O> {
    Constant(Option<O>),
    ZoomFunction(ZoomFunction<O>),
    CameraExpression(CameraExpression<O>),
}

impl<O: Copy + Interpolate> Parameter<O> {
    fn eval(&self, zoom: f32) -> Option<O> {
        match self {
            Parameter::Constant(c) => *c,
            Parameter::ZoomFunction(z) => Some(z.eval(zoom)),
            Parameter::CameraExpression(c) => Some(c.eval(zoom)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZoomFunction<T> {
    base: Option<f32>,
    stops: smallvec::SmallVec<[(f32, T); 8]>,
}

impl<T: Copy + Interpolate> ZoomFunction<T> {
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
            let mut last = steps.next().unwrap();

            for next in steps {
                if zoom >= last.0 && zoom <= next.0 {
                    let range = next.0 - last.0;
                    let start = zoom - last.0;

                    let n = start / range;

                    let base = self.base.unwrap_or(1.0);

                    if base == 1.0 {
                        return last.1.interpolate(n.powf(self.base.unwrap_or(1.0)), next.1);
                    } else {
                        let factor = (base.powf(start) - 1.0) / (base.powf(range) - 1.0);
                        return last.1.interpolate(factor, next.1);
                    }
                }

                last = next;
            }

            unreachable!()
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CameraExpression<O> {
    _marker: std::marker::PhantomData<O>,
}

impl<O> CameraExpression<O> {
    fn eval(&self, _zoom: f32) -> O {
        todo!()
    }
}
