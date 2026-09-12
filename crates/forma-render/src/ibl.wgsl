// Concatenate after shader.wgsl. Bake kernels use only group 2; the renderer
// binds empty groups 0 and 1. Each specular output view targets one array layer.
struct BakeUniforms { roughness: vec4<f32> }
@group(2) @binding(0) var bake_environment: texture_2d<f32>;
@group(2) @binding(1) var bake_output: texture_storage_2d<rgba32float, write>;
@group(2) @binding(2) var<uniform> bake_uniforms: BakeUniforms;

fn ibl_radical_inverse(input: u32) -> f32 {
    var bits = input;
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xaaaaaaaau) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xccccccccu) >> 2u);
    bits = ((bits & 0x0f0f0f0fu) << 4u) | ((bits & 0xf0f0f0f0u) >> 4u);
    bits = ((bits & 0x00ff00ffu) << 8u) | ((bits & 0xff00ff00u) >> 8u);
    return f32(bits) * 2.3283064365386963e-10;
}
fn ibl_hammersley(index: u32, count: u32) -> vec2<f32> {
    return vec2((f32(index) + 0.5) / f32(count), ibl_radical_inverse(index));
}
fn ibl_direction_uv(direction: vec3<f32>) -> vec2<f32> {
    return vec2(atan2(direction.z, direction.x) / (2.0 * PI_F) + 0.5,
        acos(clamp(direction.y, -1.0, 1.0)) * INV_PI_F);
}
fn ibl_texel_direction(pixel: vec2<u32>, size: vec2<u32>) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(size);
    let phi = (uv.x - 0.5) * (2.0 * PI_F); let theta = uv.y * PI_F;
    let sin_theta = sin(theta);
    return vec3(cos(phi) * sin_theta, cos(theta), sin(phi) * sin_theta);
}
fn ibl_source_lod(direction: vec3<f32>, pdf: f32, samples: u32) -> f32 {
    let size = textureDimensions(bake_environment, 0);
    let levels = textureNumLevels(bake_environment);
    var sin_theta = sqrt(max(0.0, 1.0 - direction.y * direction.y));
    sin_theta = max(sin_theta, sin(0.5 * PI_F / f32(size.y)));
    let texel_angle = (2.0 * PI_F / f32(size.x)) * (PI_F / f32(size.y)) * sin_theta;
    let sample_angle = 1.0 / max(f32(samples) * pdf, 1e-12);
    return clamp(0.5 * log2(max(sample_angle / texel_angle, 1.0)), 0.0, f32(levels - 1u));
}
fn ibl_read(direction: vec3<f32>, lod: f32) -> vec3<f32> {
    return max(sample_env(bake_environment, ibl_direction_uv(direction), lod).xyz, vec3(0.0));
}
@compute @workgroup_size(8, 8)
fn bake_diffuse(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel = global_id.xy; let size = textureDimensions(bake_output);
    if pixel.x >= size.x || pixel.y >= size.y { return; }
    let sample_count = 256u;
    let normal = ibl_texel_direction(pixel, size); var irradiance_over_pi = vec3(0.0);
    for (var i = 0u; i < sample_count; i += 1u) {
        let incoming = cosine_sample(ibl_hammersley(i, sample_count), normal);
        let pdf = max(dot(normal, incoming), 0.0) * INV_PI_F;
        let lod = ibl_source_lod(incoming, pdf, sample_count);
        irradiance_over_pi += ibl_read(incoming, lod);
    }
    textureStore(bake_output, vec2<i32>(pixel), vec4(irradiance_over_pi / f32(sample_count), 1.0));
}
@compute @workgroup_size(8, 8)
fn bake_specular(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel = global_id.xy; let size = textureDimensions(bake_output);
    if pixel.x >= size.x || pixel.y >= size.y { return; }
    let normal = ibl_texel_direction(pixel, size);
    let perceptual_roughness = clamp(bake_uniforms.roughness.x, 0.0, 1.0);
    if perceptual_roughness <= 1e-4 {
        textureStore(bake_output, vec2<i32>(pixel), vec4(ibl_read(normal, 0.0), 1.0));
        return;
    }
    let sample_count = 256u;
    let alpha = perceptual_roughness * perceptual_roughness; let alpha_squared = alpha * alpha;
    var radiance = vec3(0.0); var weight = 0.0;
    for (var i = 0u; i < sample_count; i += 1u) {
        let xi = ibl_hammersley(i, sample_count);
        let phi = 2.0 * PI_F * xi.x;
        let cos_theta = sqrt((1.0 - xi.y) / (1.0 + (alpha_squared - 1.0) * xi.y));
        let sin_theta = sqrt(max(0.0, 1.0 - cos_theta * cos_theta));
        let half_vector = local_to_world(vec3(cos(phi) * sin_theta, sin(phi) * sin_theta, cos_theta), normal);
        let incoming = reflect(-normal, half_vector); let n_dot_l = dot(normal, incoming);
        if n_dot_l <= 0.0 { continue; }
        let pdf = ggx_D(cos_theta, alpha) * 0.25;
        radiance += ibl_read(incoming, ibl_source_lod(incoming, pdf, sample_count)) * n_dot_l;
        weight += n_dot_l;
    }
    textureStore(bake_output, vec2<i32>(pixel), vec4(radiance / max(weight, 1e-12), 1.0));
}
@compute @workgroup_size(8, 8)
fn bake_brdf(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let pixel = global_id.xy; let size = textureDimensions(bake_output);
    if pixel.x >= size.x || pixel.y >= size.y { return; }
    let uv = (vec2<f32>(pixel) + vec2(0.5)) / vec2<f32>(size);
    let n_dot_v = uv.x; let roughness = uv.y; let alpha = roughness * roughness;
    let normal = vec3(0.0, 0.0, 1.0);
    let outgoing = vec3(sqrt(max(0.0, 1.0 - n_dot_v * n_dot_v)), 0.0, n_dot_v);
    let sample_count = 512u; var coefficients = vec2(0.0);
    for (var i = 0u; i < sample_count; i += 1u) {
        let incoming = sample_visible_ggx(outgoing, normal, alpha, ibl_hammersley(i, sample_count));
        let n_dot_l = incoming.z;
        if n_dot_l <= 0.0 { continue; }
        let half_vector = safe_normalize(outgoing + incoming, normal);
        let v_dot_h = clamp(dot(outgoing, half_vector), 0.0, 1.0);
        let one_minus = 1.0 - v_dot_h; let one_minus_squared = one_minus * one_minus;
        let fresnel_factor = one_minus_squared * one_minus_squared * one_minus;
        let visibility = ggx_G1(n_dot_l, alpha);
        coefficients += vec2(1.0 - fresnel_factor, fresnel_factor) * visibility;
    }
    textureStore(bake_output, vec2<i32>(pixel), vec4(coefficients / f32(sample_count), 0.0, 1.0));
}
