// Deterministic PBR and glass look development. All object lighting comes from the
// cached environment; scene emitters remain visible but do not illuminate peers.
// Diffuse E/pi, GGX-prefiltered reflection and the Smith GGX split-sum LUT remain
// scene-linear until the shared display transform at the end of the pixel.
#if FORMA_PREVIEW_ACCELERATED
#define PREVIEW_ACCEL_PARAM , raytracing::primitive_acceleration_structure acceleration
#define PREVIEW_ACCEL_ARG , acceleration
#else
#define PREVIEW_ACCEL_PARAM
#define PREVIEW_ACCEL_ARG
#endif

Hit preview_trace(Ray r, float limit, device const Triangle *triangles,
                  device const BvhNode *nodes, constant Uniforms &u PREVIEW_ACCEL_PARAM) {
#if FORMA_PREVIEW_ACCELERATED
    Hit hit = { limit, float2(0), NO_HIT };
    if (u.scene.x == 0u) return hit;
    raytracing::ray ray;
    ray.origin = r.origin;
    ray.direction = r.direction;
    ray.min_distance = 1.0000001e-7f;
    ray.max_distance = limit;
    raytracing::intersector<raytracing::triangle_data> intersector;
    intersector.assume_geometry_type(raytracing::geometry_type::triangle);
    intersector.force_opacity(raytracing::forced_opacity::opaque);
    intersector.set_triangle_cull_mode(raytracing::triangle_cull_mode::none);
    intersector.accept_any_intersection(false);
    auto intersection = intersector.intersect(ray, acceleration);
    if (intersection.type != raytracing::intersection_type::none && intersection.distance < limit) {
        hit.distance = intersection.distance;
        hit.bary = intersection.triangle_barycentric_coord;
        hit.triangle = intersection.primitive_id;
    }
    return hit;
#else
    return trace(r, limit, triangles, nodes, u);
#endif
}

float preview_selection_silhouette(float2 pixel, device const Triangle *triangles,
                                   device const BvhNode *nodes, constant Uniforms &u PREVIEW_ACCEL_PARAM) {
    const float2 offsets[4] = { float2(-1, 0), float2(1, 0), float2(0, -1), float2(0, 1) };
    for (uint i = 0u; i < 4u; ++i) {
        Hit neighbor = preview_trace(camera_ray(pixel + offsets[i], u), INFINITY, triangles, nodes, u PREVIEW_ACCEL_ARG);
        if (!is_selected(neighbor, triangles, u)) return 1.0f;
    }
    return 0.0f;
}

constant sampler preview_env_sampler(coord::normalized, s_address::repeat,
                                      t_address::clamp_to_edge, filter::linear,
                                      mip_filter::linear);
constant sampler preview_lut_sampler(coord::normalized, address::clamp_to_edge,
                                      filter::linear);
float3 preview_direction(float3 direction, constant Uniforms &u) {
    float c = u.preview_lighting.x, s = u.preview_lighting.y;
    return float3(c * direction.x - s * direction.z, direction.y,
                  s * direction.x + c * direction.z);
}
float2 preview_uv(float3 direction) {
    return float2(atan2(direction.z, direction.x) / (2.0f * PI_F) + 0.5f,
                  acos(clamp(direction.y, -1.0f, 1.0f)) / PI_F);
}
float3 preview_world(float3 direction, float blur, constant Uniforms &u,
                      texture2d<float, access::sample> environment) {
    if (u.preview_lighting.w > 0.5f) return max(u.world.xyz, 0.0f) * max(u.world.w, 0.0f);
    float lod = blur * float(environment.get_num_mip_levels() - 1u);
    return environment.sample(preview_env_sampler, preview_uv(preview_direction(direction, u)), level(lod)).xyz * u.preview_lighting.z;
}
float preview_contact_ao(Surface s, device const Triangle *triangles,
                         device const BvhNode *nodes, constant Uniforms &u PREVIEW_ACCEL_PARAM) {
    if (u.preview_display.z < 0.5f) return 1.0f;
    const uint sample_count = 8u;
    float occlusion = 0.0f;
    float radius = u.preview_display.w;
    for (uint i = 0u; i < sample_count; ++i) {
        // Fixed low-discrepancy directions avoid temporal noise and accumulation.
        float phi = fract(float(i) * 0.61803398875f + 0.125f);
        float3 direction = cosine_sample(float2((float(i) + 0.5f) / float(sample_count), phi), s.normal);
        if (dot(direction, s.geometric_normal) <= 0.0f) continue;
        Ray ray = { offset_origin(s.position, s.geometric_normal, direction), direction };
        Hit hit = preview_trace(ray, radius, triangles, nodes, u PREVIEW_ACCEL_ARG);
        if (hit.triangle != NO_HIT) occlusion += 1.0f - smoothstep(0.0f, radius, hit.distance);
    }
    return max(0.15f, 1.0f - occlusion / float(sample_count));
}
float3 preview_pbr(Surface s, Ray ray, float ao, constant Uniforms &u,
                   texture2d<float, access::sample> diffuse,
                   texture2d_array<float, access::sample> specular,
                   texture2d<float, access::sample> brdf) {
    float nv = max(0.0001f, dot(s.normal, -ray.direction));
    float3 f0 = mix(float3(dielectric_f0(s.ior)), s.color, s.metallic);
    float2 response = brdf.sample(preview_lut_sampler, float2(nv, s.roughness)).xy;
    float3 irradiance, reflection;
    if (u.preview_lighting.w > 0.5f) {
        irradiance = max(u.world.xyz, 0.0f) * max(u.world.w, 0.0f);
        reflection = irradiance;
    } else {
        float3 n = preview_direction(s.normal, u);
        float3 r = preview_direction(reflect(ray.direction, s.normal), u);
        irradiance = diffuse.sample(preview_env_sampler, preview_uv(n), level(0.0f)).xyz * u.preview_lighting.z;
        // Roughness is a separate texture-array dimension: even fully rough
        // metal retains the environment's directional hemispherical lighting.
        float layer = s.roughness * float(specular.get_array_size() - 1u);
        uint lower = uint(floor(layer)), upper = min(lower + 1u, specular.get_array_size() - 1u);
        float3 a = specular.sample(preview_env_sampler, preview_uv(r), lower, level(0.0f)).xyz;
        float3 b = specular.sample(preview_env_sampler, preview_uv(r), upper, level(0.0f)).xyz;
        reflection = mix(a, b, fract(layer)) * u.preview_lighting.z;
    }
    // Roughness-aware Fresnel partitions energy between opaque dielectric
    // diffuse and the single-scattering GGX reflection lobe.
    float x = 1.0f - nv, x2 = x * x;
    float3 F = f0 + (max(float3(1.0f - s.roughness), f0) - f0) * (x2 * x2 * x);
    float3 diffuse_energy = (1.0f - F) * (1.0f - s.metallic) * s.color * irradiance;
    float3 specular_energy = reflection * (f0 * response.x + response.y);
    return max((diffuse_energy + specular_energy) * ao + s.emission, 0.0f);
}

float3 preview_reflection(float3 direction, float roughness, constant Uniforms &u,
                          texture2d_array<float, access::sample> specular) {
    if (u.preview_lighting.w > 0.5f) return max(u.world.xyz, 0.0f) * max(u.world.w, 0.0f);
    float layer = roughness * float(specular.get_array_size() - 1u);
    uint lo = uint(floor(layer)), hi = min(lo + 1u, specular.get_array_size() - 1u);
    float2 uv = preview_uv(preview_direction(direction, u));
    return mix(specular.sample(preview_env_sampler, uv, lo, level(0.0f)).xyz,
               specular.sample(preview_env_sampler, uv, hi, level(0.0f)).xyz, fract(layer)) * u.preview_lighting.z;
}
float3 preview_surface(Ray ray, Hit hit, float ao, device const Triangle *triangles,
                       device const BvhNode *nodes, constant Uniforms &u, MATERIAL_ARGS,
                       texture2d<float, access::sample> environment,
                       texture2d<float, access::sample> diffuse,
                       texture2d_array<float, access::sample> specular,
                       texture2d<float, access::sample> brdf PREVIEW_ACCEL_PARAM) {
    float3 color(0), throughput(1);
    float blur = u.preview_display.y;
    // Follow glass interfaces deterministically. Reflections use the studio
    // environment; transmitted rays can reveal other scene objects.
    for (uint depth = 0u; depth < 8u; ++depth) {
        if (hit.triangle == NO_HIT) {
            float3 background = mix(viewport_background(ray), preview_world(ray.direction, blur, u, environment), u.preview_display.x);
            return color + throughput * background;
        }
        Surface s = evaluate_surface(ray, hit, triangles, u, MATERIAL_PASS);
        if (!s.glass) return color + throughput * preview_pbr(s, ray, depth == 0u ? ao : 1.0f, u, diffuse, specular, brdf);
        float eta = s.front_face ? s.ior : 1.0f / s.ior;
        float F = dielectric_fresnel(dot(s.normal, -ray.direction), eta);
        float3 transmitted = refract(ray.direction, s.normal, 1.0f / eta);
        color += throughput * s.emission;
        if (dot(transmitted, transmitted) < 1e-8f) {
            ray = { offset_origin(s.position, s.geometric_normal, reflect(ray.direction, s.normal)), reflect(ray.direction, s.normal) };
        } else {
            color += throughput * F * preview_reflection(reflect(ray.direction, s.normal), s.roughness, u, specular);
            throughput *= (1.0f - F) * s.color;
            ray = { offset_origin(s.position, s.geometric_normal, transmitted), transmitted };
        }
        blur = max(blur, s.roughness);
        hit = preview_trace(ray, INFINITY, triangles, nodes, u PREVIEW_ACCEL_ARG);
    }
    return color + throughput * preview_reflection(ray.direction, blur, u, specular);
}

kernel void preview_main(device const Triangle *triangles [[buffer(0)]],
                          device const BvhNode *nodes [[buffer(1)]],
                          constant Uniforms &u [[buffer(2)]],
                          device float4 *accumulation [[buffer(3)]],
                          device const GpuMaterial *materials [[buffer(6)]],
                          device const float4 *texture_pixels [[buffer(7)]],
                          device const uint4 *texture_levels [[buffer(8)]],
#if FORMA_PREVIEW_ACCELERATED
                          raytracing::primitive_acceleration_structure acceleration [[buffer(5)]],
#endif
                          texture2d<float, access::write> output [[texture(0)]],
                          texture2d<float, access::sample> environment [[texture(1)]],
                          texture2d<float, access::sample> diffuse [[texture(2)]],
                          texture2d_array<float, access::sample> specular [[texture(3)]],
                          texture2d<float, access::sample> brdf [[texture(4)]],
                          uint2 gid [[thread_position_in_grid]]) {
    if (gid.x >= u.image.x || gid.y >= u.image.y) return;
    float2 pixel = float2(gid) + 0.5f;
    Ray center_ray = camera_ray(pixel, u);
    Hit center_hit = preview_trace(center_ray, INFINITY, triangles, nodes, u PREVIEW_ACCEL_ARG);
    float ao = center_hit.triangle == NO_HIT ? 1.0f
        : preview_contact_ao(surface_at(center_ray, center_hit, triangles), triangles, nodes, u PREVIEW_ACCEL_ARG);
    // Fixed 2x2 spatial AA resolves silhouettes in the first completed frame.
    const float2 offsets[4] = { float2(-0.25f, -0.25f), float2(0.25f, -0.25f),
                               float2(-0.25f, 0.25f), float2(0.25f, 0.25f) };
    float3 color(0);
    for (uint i = 0u; i < 4u; ++i) {
        float2 p = pixel + offsets[i];
        Ray ray = camera_ray(p, u);
        Hit hit = preview_trace(ray, INFINITY, triangles, nodes, u PREVIEW_ACCEL_ARG);
        float3 sample;
        if (hit.triangle == NO_HIT) {
            sample = viewport_background(ray);
            // The usual neutral background needs no environment lookup or
            // equirectangular trigonometry. Illumination is independent of this.
            if (u.preview_display.x > 0.0f) {
                sample = mix(sample, preview_world(ray.direction, u.preview_display.y, u, environment), u.preview_display.x);
            }
        } else {
            bool same_object = center_hit.triangle != NO_HIT && triangles[hit.triangle].params.z == triangles[center_hit.triangle].params.z;
            sample = preview_surface(ray, hit, same_object ? ao : 1.0f, triangles, nodes, u, MATERIAL_PASS, environment, diffuse, specular, brdf PREVIEW_ACCEL_ARG);
        }
        color += add_grid(sample, ray, hit, p, u) * 0.25f;
    }
    if (is_selected(center_hit, triangles, u)) {
        float edge = preview_selection_silhouette(pixel, triangles, nodes, u PREVIEW_ACCEL_ARG);
        color = mix(color, float3(0.055f, 0.40f, 0.95f), edge * 0.92f);
    }
    accumulation[gid.y * u.image.x + gid.x] = float4(color, 1.0f);
    output.write(float4(display_transform(color, u.settings.x), 1.0f), gid);
}
