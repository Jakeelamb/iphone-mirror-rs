struct Parameters {
    // Physical viewport width/height, NV12 flag and full-range flag.
    viewport: vec4<f32>,
    screen: vec4<f32>,
    home: vec4<f32>,
    footer: vec4<f32>,
    // Screen radius, button radius, button state, physical pixels per logical pixel.
    style: vec4<f32>,
    coefficients: vec4<f32>,
};
@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var u_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> parameters: Parameters;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let positions = array<vec2<f32>, 4>(vec2(-1., 1.), vec2(-1., -1.), vec2(1., 1.), vec2(1., -1.));
    return vec4(positions[index], 0., 1.);
}

fn rounded_distance(point: vec2<f32>, rectangle: vec4<f32>, radius: f32) -> f32 {
    let half_size = rectangle.zw * 0.5;
    let offset = abs(point - rectangle.xy - half_size) - half_size + vec2(radius);
    return length(max(offset, vec2(0.))) + min(max(offset.x, offset.y), 0.) - radius;
}

fn segment_distance(point: vec2<f32>, start: vec2<f32>, end: vec2<f32>) -> f32 {
    let direction = end - start;
    let t = clamp(dot(point - start, direction) / dot(direction, direction), 0., 1.);
    return length(point - start - t * direction);
}

fn house_distance(point: vec2<f32>) -> f32 {
    var d = segment_distance(point, vec2(-7., -1.), vec2(0., -7.));
    d = min(d, segment_distance(point, vec2(0., -7.), vec2(7., -1.)));
    d = min(d, segment_distance(point, vec2(-5.5, -2.), vec2(-5.5, 6.)));
    d = min(d, segment_distance(point, vec2(-5.5, 6.), vec2(5.5, 6.)));
    d = min(d, segment_distance(point, vec2(5.5, 6.), vec2(5.5, -2.)));
    d = min(d, segment_distance(point, vec2(-1.8, 6.), vec2(-1.8, 1.)));
    d = min(d, segment_distance(point, vec2(-1.8, 1.), vec2(1.8, 1.)));
    return min(d, segment_distance(point, vec2(1.8, 1.), vec2(1.8, 6.)));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let point = position.xy;
    var rgb = vec3(0.025, 0.032, 0.044);
    if point.y >= parameters.footer.y {
        rgb = vec3(0.055, 0.067, 0.087);
        if parameters.home.z > 0. && parameters.home.w > 0. {
            let distance = rounded_distance(point, parameters.home, parameters.style.y);
            let coverage = clamp(0.5 - distance, 0., 1.);
            var button = vec3(0.13, 0.15, 0.19);
            if parameters.style.z > 1.5 {
                button = vec3(0.09, 0.12, 0.17);
            } else if parameters.style.z > 0.5 {
                button = vec3(0.20, 0.24, 0.30);
            }
            rgb = mix(rgb, button, coverage);
            let icon_point = (point - parameters.home.xy - parameters.home.zw * 0.5) / parameters.style.w;
            let icon = clamp(0.9 * parameters.style.w + 0.5 - house_distance(icon_point) * parameters.style.w, 0., 1.) * coverage;
            rgb = mix(rgb, vec3(0.88, 0.91, 0.96), icon);
        }
    } else if parameters.screen.z > 0. && parameters.screen.w > 0. {
        let distance = rounded_distance(point, parameters.screen, parameters.style.x);
        if distance < 0.5 {
            let uv = (point - parameters.screen.xy) / parameters.screen.zw;
            var y = textureSampleLevel(y_plane, plane_sampler, uv, 0.).r;
            let chroma = textureSampleLevel(u_plane, plane_sampler, uv, 0.);
            var u = chroma.r - 128. / 255.;
            var v = select(textureSampleLevel(v_plane, plane_sampler, uv, 0.).r, chroma.g, parameters.viewport.z > 0.5) - 128. / 255.;
            if parameters.viewport.w < 0.5 {
                y = (y - 16. / 255.) * 255. / 219.;
                u *= 255. / 224.;
                v *= 255. / 224.;
            }
            let c = parameters.coefficients;
            let video = vec3(y + c.x * v, y + c.y * u + c.z * v, y + c.w * u);
            rgb = mix(rgb, video, clamp(0.5 - distance, 0., 1.));
        }
    }
    return vec4(rgb, 1.);
}
