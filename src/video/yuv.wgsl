struct Parameters {
    scale: vec2<f32>,
    nv12: f32,
    full_range: f32,
    // YUV to RGB coefficients: red-V, green-U, green-V, blue-U.
    coefficients: vec4<f32>,
};
@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var u_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> parameters: Parameters;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let positions = array<vec2<f32>, 4>(vec2(-1., 1.), vec2(-1., -1.), vec2(1., 1.), vec2(1., -1.));
    let uvs = array<vec2<f32>, 4>(vec2(0., 0.), vec2(0., 1.), vec2(1., 0.), vec2(1., 1.));
    var out: VertexOutput;
    out.position = vec4(positions[index] * parameters.scale, 0., 1.);
    out.uv = uvs[index];
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var y = textureSample(y_plane, plane_sampler, in.uv).r;
    let chroma = textureSample(u_plane, plane_sampler, in.uv);
    var u = chroma.r - 128. / 255.;
    var v = select(textureSample(v_plane, plane_sampler, in.uv).r, chroma.g, parameters.nv12 > 0.5) - 128. / 255.;
    if parameters.full_range < 0.5 {
        y = (y - 16. / 255.) * 255. / 219.;
        u *= 255. / 224.;
        v *= 255. / 224.;
    }
    let c = parameters.coefficients;
    let rgb = vec3(y + c.x * v, y + c.y * u + c.z * v, y + c.w * u);
    return vec4(rgb, 1.);
}
