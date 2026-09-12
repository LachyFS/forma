// Concatenate after shader.wgsl. Deterministic PBR and glass material preview.
@group(1) @binding(0) var environment_texture: texture_2d<f32>;
@group(1) @binding(1) var diffuse_texture: texture_2d<f32>;
@group(1) @binding(2) var specular_texture: texture_2d_array<f32>;
@group(1) @binding(3) var brdf_texture: texture_2d<f32>;

fn preview_direction(direction: vec3<f32>) -> vec3<f32> {
    let c = u.preview_lighting.x; let s = u.preview_lighting.y;
    return vec3(c * direction.x - s * direction.z, direction.y, s * direction.x + c * direction.z);
}
fn preview_uv(direction: vec3<f32>) -> vec2<f32> {
    return vec2(atan2(direction.z, direction.x) / (2.0 * PI_F) + 0.5,
        acos(clamp(direction.y, -1.0, 1.0)) / PI_F);
}
fn preview_world(direction: vec3<f32>, blur: f32) -> vec3<f32> {
    if u.preview_lighting.w > 0.5 { return max(u.world.xyz, vec3(0.0)) * max(u.world.w, 0.0); }
    let lod = blur * f32(textureNumLevels(environment_texture) - 1u);
    return sample_env(environment_texture, preview_uv(preview_direction(direction)), lod).xyz * u.preview_lighting.z;
}
fn preview_contact_ao(s: Surface) -> f32 {
    if u.preview_display.z < 0.5 { return 1.0; }
    let sample_count = 8u; var occlusion = 0.0; let radius = u.preview_display.w;
    for (var i = 0u; i < sample_count; i += 1u) {
        let phi = fract(f32(i) * 0.61803398875 + 0.125);
        let direction = cosine_sample(vec2((f32(i) + 0.5) / f32(sample_count), phi), s.normal);
        if dot(direction, s.geometric_normal) <= 0.0 { continue; }
        let ray = Ray(offset_origin(s.position, s.geometric_normal, direction), direction);
        let hit = trace(ray, radius, false);
        if hit.triangle != NO_HIT { occlusion += 1.0 - smoothstep(0.0, radius, hit.distance); }
    }
    return max(0.15, 1.0 - occlusion / f32(sample_count));
}
fn preview_pbr(s: Surface, ray: Ray, ao: f32) -> vec3<f32> {
    let nv = max(0.0001, dot(s.normal, -ray.direction));
    let f0 = mix(vec3(dielectric_f0(s.ior)), s.color, s.metallic);
    let response = sample_lut(brdf_texture, vec2(nv, s.roughness)).xy;
    var irradiance: vec3<f32>; var reflection: vec3<f32>;
    if u.preview_lighting.w > 0.5 {
        irradiance = max(u.world.xyz, vec3(0.0)) * max(u.world.w, 0.0);
        reflection = irradiance;
    } else {
        let n = preview_direction(s.normal);
        let r = preview_direction(reflect(ray.direction, s.normal));
        irradiance = sample_env_level(diffuse_texture, preview_uv(n), 0).xyz * u.preview_lighting.z;
        let last = i32(textureNumLayers(specular_texture)) - 1;
        let layer = s.roughness * f32(last);
        let lower = i32(floor(layer)); let upper = min(lower + 1, last);
        let a = sample_env_array(specular_texture, preview_uv(r), lower).xyz;
        let b = sample_env_array(specular_texture, preview_uv(r), upper).xyz;
        reflection = mix(a, b, fract(layer)) * u.preview_lighting.z;
    }
    let x = 1.0 - nv; let x2 = x * x;
    let F = f0 + (max(vec3(1.0 - s.roughness), f0) - f0) * (x2 * x2 * x);
    let diffuse_energy = (vec3(1.0) - F) * (1.0 - s.metallic) * s.color * irradiance;
    let specular_energy = reflection * (f0 * response.x + vec3(response.y));
    return max((diffuse_energy + specular_energy) * ao + s.emission, vec3(0.0));
}
fn preview_reflection(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    if u.preview_lighting.w > 0.5 { return max(u.world.xyz, vec3(0.0)) * max(u.world.w, 0.0); }
    let last = i32(textureNumLayers(specular_texture)) - 1;
    let layer = roughness * f32(last); let lo = i32(floor(layer)); let hi = min(lo + 1, last);
    let uv = preview_uv(preview_direction(direction));
    return mix(sample_env_array(specular_texture, uv, lo).xyz,
               sample_env_array(specular_texture, uv, hi).xyz, fract(layer)) * u.preview_lighting.z;
}
fn preview_surface(initial_ray: Ray, initial_hit: Hit, ao: f32) -> vec3<f32> {
    var ray = initial_ray; var hit = initial_hit;
    var color = vec3(0.0); var throughput = vec3(1.0); var blur = u.preview_display.y;
    for (var depth = 0u; depth < 8u; depth += 1u) {
        if hit.triangle == NO_HIT {
            let background = mix(viewport_background(ray), preview_world(ray.direction, blur), u.preview_display.x);
            return color + throughput * background;
        }
        let s = evaluate_surface(ray, hit);
        if !s.glass { return color + throughput * preview_pbr(s, ray, select(1.0, ao, depth == 0u)); }
        let eta = select(1.0 / s.ior, s.ior, s.front_face);
        let F = dielectric_fresnel(dot(s.normal, -ray.direction), eta);
        let transmitted = refract(ray.direction, s.normal, 1.0 / eta);
        color += throughput * s.emission;
        if dot(transmitted, transmitted) < 1e-8 {
            let reflected = reflect(ray.direction, s.normal);
            ray = Ray(offset_origin(s.position, s.geometric_normal, reflected), reflected);
        } else {
            color += throughput * F * preview_reflection(reflect(ray.direction, s.normal), s.roughness);
            throughput *= (1.0 - F) * s.color;
            ray = Ray(offset_origin(s.position, s.geometric_normal, transmitted), transmitted);
        }
        blur = max(blur, s.roughness);
        hit = trace(ray, FAR, false);
    }
    return color + throughput * preview_reflection(ray.direction, blur);
}

@compute @workgroup_size(8, 8)
fn preview_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let gid = global_id.xy;
    if gid.x >= u.image.x || gid.y >= u.image.y { return; }
    let pixel = vec2<f32>(gid) + vec2(0.5);
    let center_ray = camera_ray(pixel);
    let center_hit = trace(center_ray, FAR, false);
    var ao = 1.0;
    if center_hit.triangle != NO_HIT { ao = preview_contact_ao(surface_at(center_ray, center_hit)); }
    let offsets = array<vec2<f32>, 4>(vec2(-0.25, -0.25), vec2(0.25, -0.25), vec2(-0.25, 0.25), vec2(0.25, 0.25));
    var color = vec3(0.0);
    for (var i = 0u; i < 4u; i += 1u) {
        let p = pixel + offsets[i]; let ray = camera_ray(p); let hit = trace(ray, FAR, false);
        var sample_color: vec3<f32>;
        if hit.triangle == NO_HIT {
            sample_color = viewport_background(ray);
            if u.preview_display.x > 0.0 {
                sample_color = mix(sample_color, preview_world(ray.direction, u.preview_display.y), u.preview_display.x);
            }
        } else {
            let same_object = center_hit.triangle != NO_HIT && triangles[hit.triangle].params.z == triangles[center_hit.triangle].params.z;
            sample_color = preview_surface(ray, hit, select(1.0, ao, same_object));
        }
        color += add_grid(sample_color, ray, hit, p) * 0.25;
    }
    if is_selected(center_hit) {
        color = mix(color, vec3(0.055, 0.40, 0.95), selection_silhouette(pixel) * 0.92);
    }
    accumulation[gid.y * u.image.x + gid.x] = vec4(color, 1.0);
    textureStore(output, vec2<i32>(gid), vec4(display_transform(color, u.settings.x), 1.0));
}
