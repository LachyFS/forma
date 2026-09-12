struct GpuMaterial {
    float4 color, emission, params, mapping;
    uint4 info;
    uint textures[8];
};
struct ShaderInput {
    float2 uv;
    float3 generated, position, normal, view_direction;
};
#define MATERIAL_ARGS device const GpuMaterial *materials, device const float4 *texture_pixels, device const uint4 *texture_levels
#define MATERIAL_PASS materials, texture_pixels, texture_levels

// FORMA_CUSTOM_FUNCTIONS

float2 generated_uv(float3 p, float3 face_normal, uint mapping) {
    if (mapping == 1u) {
        float3 d = safe_normalize(p - 0.5f, float3(0, 1, 0));
        return float2(atan2(d.z, d.x) / (2.0f * PI_F) + 0.5f, acos(clamp(d.y, -1.0f, 1.0f)) / PI_F);
    }
    if (mapping == 2u) return float2(p.x, 1.0f - p.z);
    float3 a = abs(face_normal);
    if (a.x >= a.y && a.x >= a.z) return float2(face_normal.x >= 0 ? 1.0f - p.z : p.z, 1.0f - p.y);
    if (a.y >= a.z) return float2(p.x, face_normal.y >= 0 ? 1.0f - p.z : p.z);
    return float2(face_normal.z >= 0 ? p.x : 1.0f - p.x, 1.0f - p.y);
}
uint wrap_texel(int p, uint size) { int n = int(size); return uint((p % n + n) % n); }
float4 bilinear_texture(uint4 desc, float2 uv, device const float4 *pixels) {
    float2 p = fract(uv) * float2(desc.yz) - 0.5f;
    int2 a = int2(floor(p)); float2 f = fract(p);
    uint x0 = wrap_texel(a.x, desc.y), x1 = wrap_texel(a.x + 1, desc.y);
    uint y0 = wrap_texel(a.y, desc.z), y1 = wrap_texel(a.y + 1, desc.z);
    return mix(mix(pixels[desc.x + y0 * desc.y + x0], pixels[desc.x + y0 * desc.y + x1], f.x),
               mix(pixels[desc.x + y1 * desc.y + x0], pixels[desc.x + y1 * desc.y + x1], f.x), f.y);
}
float4 image_texture(uint index, float2 uv, float footprint, device const float4 *pixels, device const uint4 *levels) {
    if (index == NO_HIT) return float4(1);
    uint4 desc = levels[index];
    float lod = clamp(log2(max(1.0f, footprint * float(max(desc.y, desc.z)))), 0.0f, float(desc.w - 1u));
    uint lo = uint(floor(lod)), hi = min(lo + 1u, desc.w - 1u);
    return mix(bilinear_texture(levels[index + lo], uv, pixels), bilinear_texture(levels[index + hi], uv, pixels), fract(lod));
}
Surface evaluate_surface(Ray ray, Hit hit, device const Triangle *triangles, constant Uniforms &u, MATERIAL_ARGS) {
    Surface s = surface_at(ray, hit, triangles);
    device const Triangle &tri = triangles[hit.triangle];
    device const GpuMaterial &m = materials[uint(tri.generated0.w)];
    float3 weights(1.0f - hit.bary.x - hit.bary.y, hit.bary.x, hit.bary.y);
    float3 q0 = tri.generated0.xyz, q1 = tri.generated1.xyz, q2 = tri.generated2.xyz;
    float3 generated = q0 * weights.x + q1 * weights.y + q2 * weights.z;
    float3 face = safe_normalize(cross(q1 - q0, q2 - q0), float3(0, 1, 0));
    float2 uv = generated_uv(generated, face, m.info.y) * m.mapping.xy + m.mapping.zw;
    float2 uv0 = generated_uv(q0, face, m.info.y);
    float2 uv1 = generated_uv(q1, face, m.info.y);
    float2 uv2 = generated_uv(q2, face, m.info.y);
    if (m.info.y == 1u) {
        // Unwrap triangle derivatives across the spherical longitude seam.
        uv1.x -= round(uv1.x - uv0.x); uv2.x -= round(uv2.x - uv0.x);
    }
    float2 d1 = (uv1 - uv0) * m.mapping.xy, d2 = (uv2 - uv0) * m.mapping.xy;
    float3 e1 = tri.v1.xyz - tri.v0.xyz, e2 = tri.v2.xyz - tri.v0.xyz;
    float density = max(length(d1) / max(length(e1), 1e-6f), length(d2) / max(length(e2), 1e-6f));
    float pixel_width = 2.0f * (u.camera.w > 0.5f ? u.camera.z : hit.distance * u.camera.x) / float(u.image.y);
    float footprint = density * pixel_width / max(0.15f, abs(dot(s.normal, ray.direction)));
    s.color = m.color.xyz * image_texture(m.textures[0], uv, footprint, texture_pixels, texture_levels).xyz;
    s.emission = m.emission.xyz * image_texture(m.textures[4], uv, footprint, texture_pixels, texture_levels).xyz;
    s.metallic = m.textures[2] == NO_HIT ? m.params.x : image_texture(m.textures[2], uv, footprint, texture_pixels, texture_levels).x;
    s.roughness = m.textures[1] == NO_HIT ? m.params.y : image_texture(m.textures[1], uv, footprint, texture_pixels, texture_levels).x;
    s.ior = m.params.z; s.glass = m.info.x == 1u;
    float determinant = d1.x * d2.y - d1.y * d2.x;
    if (m.textures[3] != NO_HIT && abs(determinant) > 1e-8f) {
        float3 t = (e1 * d2.y - e2 * d1.y) / determinant;
        t = safe_normalize(t - s.normal * dot(t, s.normal), float3(1, 0, 0));
        float3 b = cross(s.normal, t);
        // Image rows run downward, whereas tangent-space normal +Y runs up.
        float3 dpdv = (e2 * d1.x - e1 * d2.x) / determinant;
        if (dot(b, dpdv) > 0.0f) b = -b;
        float3 map = image_texture(m.textures[3], uv, footprint, texture_pixels, texture_levels).xyz * 2.0f - 1.0f;
        map.xy *= m.params.w;
        s.normal = safe_normalize(t * map.x + b * map.y + s.normal * map.z, s.normal);
    }
    Surface before = s;
    if (m.info.x == 2u) {
        ShaderInput input = { uv, generated, s.position, s.normal, -ray.direction };
        forma_custom(m.info.z, s, input);
    }
    // A bad numerical result affects this surface only, never the film/history.
    s.position = before.position; s.geometric_normal = before.geometric_normal; s.front_face = before.front_face;
    s.color = all(isfinite(s.color)) ? clamp(s.color, 0.0f, 1.0f) : before.color;
    s.emission = all(isfinite(s.emission)) ? clamp(s.emission, 0.0f, 1e6f) : before.emission;
    s.metallic = isfinite(s.metallic) ? clamp(s.metallic, 0.0f, 1.0f) : m.params.x;
    s.roughness = isfinite(s.roughness) ? clamp(s.roughness, 0.025f, 1.0f) : max(0.025f, m.params.y);
    s.ior = isfinite(s.ior) ? clamp(s.ior, 1.01f, 3.0f) : m.params.z;
    s.normal = all(isfinite(s.normal)) ? safe_normalize(s.normal, s.geometric_normal) : s.geometric_normal;
    if (dot(s.normal, s.geometric_normal) < 1e-4f || dot(s.normal, -ray.direction) < 1e-4f) s.normal = s.geometric_normal;
    return s;
}
