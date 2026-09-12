// This source is compiled after shader.metal and shares its GGX implementation.
// The bake is deterministic and independent of the current scene or camera.

float ibl_radical_inverse(uint bits) {
    bits = (bits << 16u) | (bits >> 16u);
    bits = ((bits & 0x55555555u) << 1u) | ((bits & 0xaaaaaaaau) >> 1u);
    bits = ((bits & 0x33333333u) << 2u) | ((bits & 0xccccccccu) >> 2u);
    bits = ((bits & 0x0f0f0f0fu) << 4u) | ((bits & 0xf0f0f0f0u) >> 4u);
    bits = ((bits & 0x00ff00ffu) << 8u) | ((bits & 0xff00ff00u) >> 8u);
    return float(bits) * 2.3283064365386963e-10f;
}

float2 ibl_hammersley(uint index, uint count) {
    return float2((float(index) + 0.5f) / float(count), ibl_radical_inverse(index));
}

float2 ibl_direction_uv(float3 direction) {
    return float2(atan2(direction.z, direction.x) / (2.0f * PI_F) + 0.5f,
                  acos(clamp(direction.y, -1.0f, 1.0f)) * INV_PI_F);
}

float3 ibl_texel_direction(uint2 pixel, uint width, uint height) {
    float2 uv = (float2(pixel) + 0.5f) / float2(width, height);
    float phi = (uv.x - 0.5f) * (2.0f * PI_F), theta = uv.y * PI_F;
    float sin_theta = sin(theta);
    return float3(cos(phi) * sin_theta, cos(theta), sin(phi) * sin_theta);
}

float ibl_source_lod(float3 direction, float pdf, uint samples,
                     uint width, uint height, uint levels) {
    // An equirectangular texel's solid angle shrinks towards the poles. Use the
    // local solid angle, rather than the constant cubemap approximation, when
    // choosing the source footprint for a Monte Carlo sample.
    float sin_theta = sqrt(max(0.0f, 1.0f - direction.y * direction.y));
    sin_theta = max(sin_theta, sin(0.5f * PI_F / float(height)));
    float texel_angle = (2.0f * PI_F / float(width)) * (PI_F / float(height)) * sin_theta;
    float sample_angle = 1.0f / max(float(samples) * pdf, 1e-12f);
    return clamp(0.5f * log2(max(sample_angle / texel_angle, 1.0f)),
                 0.0f, float(levels - 1u));
}

float3 ibl_read(texture2d<float, access::sample> environment, float3 direction, float lod) {
    constexpr sampler environment_sampler(coord::normalized, s_address::repeat,
        t_address::clamp_to_edge, min_filter::linear, mag_filter::linear, mip_filter::linear);
    return max(environment.sample(environment_sampler, ibl_direction_uv(direction), level(lod)).rgb,
               float3(0.0f));
}

kernel void bake_diffuse(texture2d<float, access::sample> environment [[texture(0)]],
                         texture2d<float, access::write> output [[texture(1)]],
                         uint2 pixel [[thread_position_in_grid]]) {
    if (pixel.x >= output.get_width() || pixel.y >= output.get_height()) return;
    constexpr uint sample_count = 256u;
    float3 normal = ibl_texel_direction(pixel, output.get_width(), output.get_height());
    float3 irradiance_over_pi(0.0f);
    for (uint i = 0u; i < sample_count; ++i) {
        float3 incoming = cosine_sample(ibl_hammersley(i, sample_count), normal);
        float pdf = max(dot(normal, incoming), 0.0f) * INV_PI_F;
        float lod = ibl_source_lod(incoming, pdf, sample_count, environment.get_width(),
                                   environment.get_height(), environment.get_num_mip_levels());
        // Cosine sampling cancels cos(theta)/pi; the mean is irradiance/pi.
        irradiance_over_pi += ibl_read(environment, incoming, lod);
    }
    output.write(float4(irradiance_over_pi / float(sample_count), 1.0f), pixel);
}

kernel void bake_specular(texture2d<float, access::sample> environment [[texture(0)]],
                          texture2d<float, access::write> output [[texture(1)]],
                          constant float &roughness [[buffer(0)]],
                          uint2 pixel [[thread_position_in_grid]]) {
    if (pixel.x >= output.get_width() || pixel.y >= output.get_height()) return;
    float3 normal = ibl_texel_direction(pixel, output.get_width(), output.get_height());
    float perceptual_roughness = clamp(roughness, 0.0f, 1.0f);
    if (perceptual_roughness <= 1e-4f) {
        output.write(float4(ibl_read(environment, normal, 0.0f), 1.0f), pixel);
        return;
    }

    constexpr uint sample_count = 256u;
    float alpha = perceptual_roughness * perceptual_roughness;
    float alpha_squared = alpha * alpha;
    float3 radiance(0.0f);
    float weight = 0.0f;
    for (uint i = 0u; i < sample_count; ++i) {
        float2 xi = ibl_hammersley(i, sample_count);
        float phi = 2.0f * PI_F * xi.x;
        float cos_theta = sqrt((1.0f - xi.y) / (1.0f + (alpha_squared - 1.0f) * xi.y));
        float sin_theta = sqrt(max(0.0f, 1.0f - cos_theta * cos_theta));
        float3 half_vector = local_to_world(float3(cos(phi) * sin_theta, sin(phi) * sin_theta,
                                                   cos_theta), normal);
        float3 incoming = reflect(-normal, half_vector);
        float n_dot_l = dot(normal, incoming);
        if (n_dot_l <= 0.0f) continue;
        // V=N in the split-sum prefilter, so D(N.H)*(N.H)/(4*V.H) = D/4.
        float pdf = ggx_D(cos_theta, alpha) * 0.25f;
        float lod = ibl_source_lod(incoming, pdf, sample_count, environment.get_width(),
                                   environment.get_height(), environment.get_num_mip_levels());
        radiance += ibl_read(environment, incoming, lod) * n_dot_l;
        weight += n_dot_l;
    }
    // Normalization preserves constant environments at every roughness.
    output.write(float4(radiance / max(weight, 1e-12f), 1.0f), pixel);
}

kernel void bake_brdf(texture2d<float, access::write> output [[texture(0)]],
                       uint2 pixel [[thread_position_in_grid]]) {
    if (pixel.x >= output.get_width() || pixel.y >= output.get_height()) return;
    float2 uv = (float2(pixel) + 0.5f) / float2(output.get_width(), output.get_height());
    float n_dot_v = uv.x, roughness = uv.y;
    float alpha = roughness * roughness;
    float3 normal(0.0f, 0.0f, 1.0f);
    float3 outgoing(sqrt(max(0.0f, 1.0f - n_dot_v * n_dot_v)), 0.0f, n_dot_v);
    constexpr uint sample_count = 512u;
    float2 coefficients(0.0f);
    for (uint i = 0u; i < sample_count; ++i) {
        float3 incoming = sample_visible_ggx(outgoing, normal, alpha, ibl_hammersley(i, sample_count));
        float n_dot_l = incoming.z;
        if (n_dot_l <= 0.0f) continue;
        float3 half_vector = safe_normalize(outgoing + incoming, normal);
        float v_dot_h = clamp(dot(outgoing, half_vector), 0.0f, 1.0f);
        float one_minus = 1.0f - v_dot_h;
        float one_minus_squared = one_minus * one_minus;
        float fresnel_factor = one_minus_squared * one_minus_squared * one_minus;
        // Visible-normal PDF = D*G1(V)/(4*N.V). Dividing BRDF*cos by
        // this PDF leaves F*G1(L), with the exact Smith term used at runtime.
        float visibility = ggx_G1(n_dot_l, alpha);
        coefficients += float2(1.0f - fresnel_factor, fresnel_factor) * visibility;
    }
    output.write(float4(coefficients / float(sample_count), 0.0f, 1.0f), pixel);
}
