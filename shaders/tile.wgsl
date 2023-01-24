struct TileConstants {
    scale: f32,
    line_width: f32,
    offset: vec2<f32>,
    tile_dims: vec2<f32>,
    window_dims: vec2<f32>,
    fill_translate: vec2<f32>,
    line_translate: vec2<f32>,
    fill_color: vec4<f32>,
    line_color: vec4<f32>
}

var<push_constant> tile_constants: TileConstants;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) fill: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(tile: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    var a_position: vec2<f32>;

    if tile.fill == 1u {
        out.color = tile_constants.fill_color;
        a_position = tile.position + (tile_constants.fill_translate / tile_constants.scale);
    } else if tile.fill == 0u {
        out.color = tile_constants.line_color;
        a_position = tile.position + (tile.normal * tile_constants.line_width / 2.0 / tile_constants.scale);
        a_position = a_position + (tile_constants.line_translate / tile_constants.scale);
    } else if tile.fill == 2u {
        out.color = vec4(0.6, 0.6, 0.6, 1.0);
        a_position = tile.position + (tile.normal * (1.3 / 64.0) / 2.0 / tile_constants.scale);
    } else if tile.fill == 3u {
        out.color = vec4(0.95, 0.95, 0.95, 1.0);
        a_position = tile.position + (tile.normal * (0.85 / 64.0) / 2.0 / tile_constants.scale);
    }

    var a_offset = tile_constants.offset;
    a_position = (a_position * (tile_constants.tile_dims)) + a_offset;
    a_position = a_position / tile_constants.window_dims;
    a_position = (a_position * 2.0) - 1.0;
    a_position = a_position * vec2(1.0, -1.0);

    out.position = vec4(a_position, 1.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4(pow(in.color.xyz, vec3(2.2)), in.color.w);
}

