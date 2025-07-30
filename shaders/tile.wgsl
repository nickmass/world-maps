struct TileConstants {
    fill_translate: vec2<f32>,
    line_translate: vec2<f32>,
    fill_color: vec4<f32>,
    line_color: vec4<f32>,
    transform: mat3x3<f32>, // padded to mat3x4
    line_width: f32,
    line_dasharray: array<f32, 8>,
    line_dasharray_len: u32,
    line_dasharray_total: f32,
    rescale_scale: f32,
    rescale_offset: vec2<f32>
}

var<push_constant> tile_constants: TileConstants;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) advancement: f32,
    @location(3) fill: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(linear) advancement: f32,
}

const FILL_LINE: u32 = 0;
const FILL_POLYGON: u32 = 1;
const FILL_BACKGROUND: u32 = 2;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var tile = input;
    tile.position = (tile.position - tile_constants.rescale_offset) * tile_constants.rescale_scale;

    var out: VertexOutput;
    var position: vec2<f32>;

    switch tile.fill {
        case FILL_LINE {
          out.color = tile_constants.line_color;
          let line_width = tile.normal * (tile_constants.line_width / 2.0);
          position = tile.position + line_width + tile_constants.line_translate;
        }
        case FILL_POLYGON {
          out.color = tile_constants.fill_color;
          position = tile.position + tile_constants.fill_translate;
        }
        case FILL_BACKGROUND {
          out.color = tile_constants.fill_color;
          position = tile.position + tile_constants.fill_translate;
        }
        default: {
          out.color = vec4(1.0, 0.0, 1.0, 1.0);
          position = tile.position;
        }
    }

    out.position = vec4(tile_constants.transform * vec3(position, 1.0), 1.0);
    out.advancement = tile.advancement;

    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    if tile_constants.line_dasharray_len > 0 {
        var dash_len = floor(in.advancement / tile_constants.line_dasharray_total) * tile_constants.line_dasharray_total;
        for (var i = 0u; i < tile_constants.line_dasharray_len; i++) {
            dash_len += tile_constants.line_dasharray[i];
            if dash_len >= in.advancement {
                if i % 2 == 1 {
                    discard;
                }
                break;
            }
        }
    }

    return vec4(pow(in.color.xyz, vec3(2.2)), in.color.w);
}
