struct TextConstants {
    scale: f32,
    halo_width: f32,
    offset: vec2<f32>,
    tile_dims: vec2<f32>,
    window_dims: vec2<f32>,
    text_color: vec4<f32>,
    halo_color: vec4<f32>
}

var<push_constant> text_constants: TextConstants;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) label_offset: vec2<f32>,
    @location(3) halo: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(text: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    var label_offset = text.label_offset * text_constants.tile_dims;
    var tile_offset = text_constants.offset;

    var offset_y = tile_offset.y + label_offset.y - text.position.y;
    var offset_x = tile_offset.x + label_offset.x + text.position.x;
    var offset = vec2(offset_x, offset_y);

    let width = text_constants.halo_width;
    if text.halo == 1u {
        out.color = text_constants.halo_color;
        offset = offset + width;
    } else if text.halo == 2u {
        out.color = text_constants.halo_color;
        offset = vec2(offset.x - width, offset.y + width);
    } else if text.halo == 3u {
        out.color = text_constants.halo_color;
        offset = vec2(offset.x + width, offset.y - width);
    } else if text.halo == 4u {
        out.color = text_constants.halo_color;
        offset = offset - width;
    } else {
        out.color = text_constants.text_color;
    }

    var a_position = vec2(offset / text_constants.window_dims);
    a_position = vec2(a_position.x, 1.0 - a_position.y);
    a_position = a_position * 2.0 - 1.0;

    out.position = vec4(a_position, 1.0, 1.0);
    out.uv = text.uv;
    return out;
}

@group(0) @binding(0) var t_text_atlas: texture_2d<f32>;
@group(0) @binding(1) var s_text_atlas: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var coverage = textureSample(t_text_atlas, s_text_atlas, in.uv).x;

    return vec4(pow(in.color.xyz, vec3(2.2)), in.color.w * coverage);
}

