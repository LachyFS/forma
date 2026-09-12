#include <metal_stdlib>
using namespace metal;

// Metallic/roughness surfaces: energy-partitioned Lambert +
// single-scattering GGX with Schlick Fresnel (dielectric IOR 1.5). Emission is
// two-sided radiance. Glass uses a rough dielectric reflection/transmission BSDF.
// Rendering uses next-event estimation for area emitters AND the environment,
// with a power-heuristic MIS weight on their complementary BSDF paths. The
// deterministic Material Preview pipeline is implemented in preview.metal.
struct Triangle {
    float4 v0, v1, v2;
    float4 n0, n1, n2;
    float4 base_color, emission;
    float4 params; // metallic, roughness, object ID, original-edge mask
    float4 generated0, generated1, generated2; // object coordinates; material index in generated0.w
};
struct BvhNode {
    float4 minimum, maximum;
    uint4 data; // left/start, right/count, is_leaf, reserved
};
struct Uniforms {
    float4 cam_origin, cam_right, cam_up, cam_forward;
    float4 world; // linear RGB, intensity
    float4 camera; // tan(vertical FOV / 2), aspect, ortho half-height, is_ortho
    uint4 image; // width, height, zero-based sample index, shading mode
    uint4 scene; // triangle count, node count, emissive triangle count, bounces
    float4 settings; // exposure stops, grid, selected object ID (0 = none), reserved
    float4 preview_lighting; // cos(rotation), sin(rotation), intensity, use scene world
    float4 preview_display; // world opacity, background blur, contact AO, AO distance
};
struct Ray { float3 origin, direction; };
struct Hit {
    float distance;
    float2 bary;
    uint triangle;
};
struct Surface {
    float3 position, normal, geometric_normal, color, emission;
    float metallic, roughness, ior;
    bool front_face, glass;
};
struct BsdfSample { float3 direction, value; float pdf; };
constant float PI_F = 3.14159265358979323846f;
constant float INV_PI_F = 0.31830988618379067154f;
constant uint NO_HIT = 0xffffffffu;

uint hash_u32(uint x) {
    x ^= x >> 16; x *= 0x7feb352du;
    x ^= x >> 15; x *= 0x846ca68bu;
    return x ^ (x >> 16);
}
float random_float(thread uint &state) {
    state = state * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return float(((word >> 22u) ^ word) >> 8u) * (1.0f / 16777216.0f);
}
float2 random_pair(thread uint &state) {
    float x = random_float(state);
    return float2(x, random_float(state));
}
float luminance(float3 v) { return dot(v, float3(0.2126f, 0.7152f, 0.0722f)); }
float max_component(float3 v) { return max(v.x, max(v.y, v.z)); }
float3 safe_normalize(float3 v, float3 fallback) {
    float len2 = dot(v, v);
    return len2 > 1e-20f ? v * rsqrt(len2) : fallback;
}
void basis(float3 n, thread float3 &t, thread float3 &b) {
    t = safe_normalize(cross(abs(n.y) < 0.999f ? float3(0, 1, 0) : float3(1, 0, 0), n), float3(1, 0, 0));
    b = cross(n, t);
}
float3 local_to_world(float3 v, float3 n) {
    float3 t, b; basis(n, t, b);
    return t * v.x + b * v.y + n * v.z;
}
float3 cosine_sample(float2 u, float3 n) {
    float r = sqrt(u.x), phi = 2.0f * PI_F * u.y;
    return local_to_world(float3(r * cos(phi), r * sin(phi), sqrt(max(0.0f, 1.0f - u.x))), n);
}
float power_heuristic(float a, float b) {
    // Ratio form avoids squaring very large solid-angle light PDFs.
    if (a <= 0.0f) return 0.0f;
    float ratio = b / a;
    return 1.0f / (1.0f + ratio * ratio);
}

// Integer-ULP origin offset, with a small absolute offset close to the origin.
// This follows the scale-independent floating-point offset construction in
// Ray Tracing Gems; offsets follow the geometric normal, never a smooth normal.
float offset_component(float p, float n) {
    int offset = int(256.0f * n);
    float shifted = as_type<float>(as_type<int>(p) + (p < 0.0f ? -offset : offset));
    return abs(p) < (1.0f / 32.0f) ? p + n * (1.0f / 65536.0f) : shifted;
}
float3 offset_origin(float3 p, float3 normal, float3 outgoing) {
    float3 n = dot(normal, outgoing) >= 0.0f ? normal : -normal;
    return float3(offset_component(p.x, n.x), offset_component(p.y, n.y), offset_component(p.z, n.z));
}
float inverse_component(float v) {
    return abs(v) < 1e-30f ? (v < 0.0f ? -1e30f : 1e30f) : 1.0f / v;
}
float box_distance(Ray ray, float3 inverse_dir, device const BvhNode &node, float limit) {
    float3 a = (node.minimum.xyz - ray.origin) * inverse_dir;
    float3 b = (node.maximum.xyz - ray.origin) * inverse_dir;
    float3 lo = min(a, b), hi = max(a, b);
    float near_t = max(0.0f, max_component(lo));
    float far_t = min(hi.x, min(hi.y, hi.z));
    return far_t >= near_t && near_t < limit ? near_t : INFINITY;
}
bool triangle_hit(Ray ray, device const Triangle &tri, thread Hit &hit, uint index) {
    float3 e1 = tri.v1.xyz - tri.v0.xyz, e2 = tri.v2.xyz - tri.v0.xyz;
    float3 p = cross(ray.direction, e2);
    float determinant = dot(e1, p);
    float scale = sqrt(dot(e1, e1) * dot(e2, e2));
    if (abs(determinant) <= max(1e-30f, scale * 1e-8f)) return false;
    float inv = 1.0f / determinant;
    float3 t = ray.origin - tri.v0.xyz;
    float u = dot(t, p) * inv;
    if (u < 0.0f || u > 1.0f) return false;
    float3 q = cross(t, e1);
    float v = dot(ray.direction, q) * inv;
    if (v < 0.0f || u + v > 1.0f) return false;
    float distance = dot(e2, q) * inv;
    if (distance <= 1e-7f || distance >= hit.distance) return false;
    hit.distance = distance; hit.bary = float2(u, v); hit.triangle = index;
    return true;
}
Hit trace(Ray ray, float limit, device const Triangle *triangles,
          device const BvhNode *nodes, constant Uniforms &u, bool any_hit = false) {
    Hit hit = { limit, float2(0), NO_HIT };
    if (u.scene.x == 0u || u.scene.y == 0u) return hit;
    float3 inverse_dir(inverse_component(ray.direction.x), inverse_component(ray.direction.y), inverse_component(ray.direction.z));
    uint stack[64]; uint stack_size = 1u; stack[0] = 0u;
    bool overflow = false;
    while (stack_size > 0u) {
        uint index = stack[--stack_size];
        if (index >= u.scene.y) continue;
        device const BvhNode &node = nodes[index];
        if (!isfinite(box_distance(ray, inverse_dir, node, hit.distance))) continue;
        if (node.data.z != 0u) {
            uint end = min(u.scene.x, node.data.x + node.data.y);
            for (uint i = node.data.x; i < end; ++i) {
                if (triangle_hit(ray, triangles[i], hit, i) && any_hit) return hit;
            }
        } else {
            uint left = node.data.x, right = node.data.y;
            float dl = left < u.scene.y ? box_distance(ray, inverse_dir, nodes[left], hit.distance) : INFINITY;
            float dr = right < u.scene.y ? box_distance(ray, inverse_dir, nodes[right], hit.distance) : INFINITY;
            uint near_node = dl < dr ? left : right, far_node = dl < dr ? right : left;
            float near_dist = min(dl, dr), far_dist = max(dl, dr);
            uint needed = uint(isfinite(near_dist)) + uint(isfinite(far_dist));
            if (stack_size + needed > 64u) { overflow = true; break; }
            if (isfinite(far_dist)) stack[stack_size++] = far_node;
            if (isfinite(near_dist)) stack[stack_size++] = near_node;
        }
    }
    // A pathological imported BVH must not silently drop visible geometry.
    if (overflow) {
        for (uint i = 0u; i < u.scene.x; ++i)
            if (triangle_hit(ray, triangles[i], hit, i) && any_hit) break;
    }
    return hit;
}
Surface surface_at(Ray ray, Hit hit, device const Triangle *triangles) {
    device const Triangle &tri = triangles[hit.triangle];
    float3 weights(1.0f - hit.bary.x - hit.bary.y, hit.bary.x, hit.bary.y);
    float3 geometric = safe_normalize(cross(tri.v1.xyz - tri.v0.xyz, tri.v2.xyz - tri.v0.xyz), float3(0, 1, 0));
    float3 normal = safe_normalize(weights.x * tri.n0.xyz + weights.y * tri.n1.xyz + weights.z * tri.n2.xyz, geometric);
    if (dot(normal, geometric) < 0.0f) geometric = -geometric;
    bool front_face = dot(geometric, ray.direction) < 0.0f;
    if (dot(geometric, ray.direction) > 0.0f) { geometric = -geometric; normal = -normal; }
    if (dot(normal, -ray.direction) < 1e-4f) normal = geometric;
    Surface result;
    result.front_face = front_face; result.glass = false; result.ior = 1.5f;
    result.position = ray.origin + ray.direction * hit.distance;
    result.normal = normal; result.geometric_normal = geometric;
    result.color = clamp(tri.base_color.xyz, 0.0f, 1.0f);
    result.emission = max(tri.emission.xyz, 0.0f);
    result.metallic = clamp(tri.params.x, 0.0f, 1.0f);
    result.roughness = clamp(tri.params.y, 0.025f, 1.0f);
    return result;
}
// FORMA_MATERIAL_SYSTEM

float dielectric_f0(float ior) { float r = (ior - 1.0f) / (ior + 1.0f); return r * r; }
float3 fresnel(float cos_theta, float3 f0) {
    float x = clamp(1.0f - cos_theta, 0.0f, 1.0f);
    float x2 = x * x;
    return f0 + (1.0f - f0) * (x2 * x2 * x);
}
float ggx_D(float n_dot_h, float alpha) {
    float a2 = alpha * alpha;
    float d = n_dot_h * n_dot_h * (a2 - 1.0f) + 1.0f;
    return a2 / (PI_F * d * d);
}
float ggx_G1(float n_dot_v, float alpha) {
    float a2 = alpha * alpha;
    return (2.0f * n_dot_v) / max(1e-12f, n_dot_v + sqrt(a2 + (1.0f - a2) * n_dot_v * n_dot_v));
}
// Exact dielectric Fresnel, including total internal reflection. eta is the
// destination/source IOR ratio. The normal always faces the incident ray.
float dielectric_fresnel(float cosine, float eta) {
    float c = clamp(abs(cosine), 0.0f, 1.0f);
    float sin2_t = (1.0f - c * c) / (eta * eta);
    if (sin2_t >= 1.0f) return 1.0f;
    float ct = sqrt(max(0.0f, 1.0f - sin2_t));
    float rs = (c - eta * ct) / max(1e-12f, c + eta * ct);
    float rp = (eta * c - ct) / max(1e-12f, eta * c + ct);
    return 0.5f * (rs * rs + rp * rp);
}
// GGX rough dielectric with matching visible-normal sampling/PDF and the
// radiance-mode eta correction; see PBRT 4e, Dielectric BSDF.
float3 evaluate_glass(Surface s, float3 wo, float3 wi, thread float &pdf) {
    pdf = 0.0f;
    float nv = dot(s.normal, wo), nl = dot(s.normal, wi);
    if (nv <= 0.0f || abs(nl) < 1e-7f || nl * dot(s.geometric_normal, wi) <= 0.0f) return float3(0);
    bool reflection = nl > 0.0f;
    float eta = s.front_face ? s.ior : 1.0f / s.ior;
    float3 h = safe_normalize(wo + wi * (reflection ? 1.0f : eta), s.normal);
    if (dot(h, s.normal) < 0.0f) h = -h;
    float vh = dot(wo, h), lh = dot(wi, h);
    if (vh <= 0.0f || lh * nl <= 0.0f) return float3(0);
    float alpha = s.roughness * s.roughness;
    float D = ggx_D(max(0.0f, dot(s.normal, h)), alpha);
    float Gv = ggx_G1(nv, alpha), Gl = ggx_G1(abs(nl), alpha);
    float F = dielectric_fresnel(vh, eta);
    float half_pdf = D * Gv * vh / nv;
    if (reflection) {
        pdf = half_pdf * F / (4.0f * vh);
        return float3(F * D * Gv * Gl / max(1e-12f, 4.0f * nv * nl));
    }
    float d = lh + vh / eta, denom = max(1e-16f, d * d);
    pdf = half_pdf * (1.0f - F) * abs(lh) / denom;
    return s.color * ((1.0f - F) * D * Gv * Gl * abs(lh * vh / (nl * nv)) / (denom * eta * eta));
}

float specular_probability(Surface s, float3 wo) {
    float3 f0 = mix(float3(dielectric_f0(s.ior)), s.color, s.metallic);
    float reflectance = luminance(fresnel(max(0.0f, dot(s.normal, wo)), f0));
    float diffuse = luminance(s.color) * (1.0f - s.metallic);
    return s.metallic > 0.999f ? 1.0f : clamp(reflectance / max(1e-6f, reflectance + diffuse), 0.1f, 0.95f);
}
float3 evaluate_bsdf(Surface s, float3 wo, float3 wi, thread float &pdf) {
    if (s.glass) return evaluate_glass(s, wo, wi, pdf);
    pdf = 0.0f;
    float nv = dot(s.normal, wo), nl = dot(s.normal, wi);
    if (nv <= 0.0f || nl <= 0.0f || dot(s.geometric_normal, wi) <= 0.0f) return float3(0);
    float3 h = safe_normalize(wo + wi, s.normal);
    float nh = max(0.0f, dot(s.normal, h)), vh = max(0.0f, dot(wo, h));
    float alpha = s.roughness * s.roughness;
    float D = ggx_D(nh, alpha), Gv = ggx_G1(nv, alpha), Gl = ggx_G1(nl, alpha);
    float3 F = fresnel(vh, mix(float3(dielectric_f0(s.ior)), s.color, s.metallic));
    float3 specular = F * (D * Gv * Gl / max(1e-12f, 4.0f * nv * nl));
    float3 diffuse = (1.0f - F) * (1.0f - s.metallic) * s.color * INV_PI_F;
    float p_spec = specular_probability(s, wo);
    // Visible-normal GGX sampling: p(h) = D(h) G1(v) (v.h)/(n.v).
    float spec_pdf = D * Gv / max(1e-12f, 4.0f * nv);
    pdf = mix(nl * INV_PI_F, spec_pdf, p_spec);
    return diffuse + specular;
}
float3 sample_visible_ggx(float3 wo, float3 n, float alpha, float2 u) {
    float3 t, b; basis(n, t, b);
    float3 v(dot(wo, t), dot(wo, b), dot(wo, n));
    float3 vh = safe_normalize(float3(alpha * v.x, alpha * v.y, v.z), float3(0, 0, 1));
    float len2 = vh.x * vh.x + vh.y * vh.y;
    float3 t1 = len2 > 0.0f ? float3(-vh.y, vh.x, 0.0f) * rsqrt(len2) : float3(1, 0, 0);
    float3 t2 = cross(vh, t1);
    float r = sqrt(u.x), phi = 2.0f * PI_F * u.y;
    float p1 = r * cos(phi), p2 = r * sin(phi);
    float blend = 0.5f * (1.0f + vh.z);
    p2 = (1.0f - blend) * sqrt(max(0.0f, 1.0f - p1 * p1)) + blend * p2;
    float3 nh = p1 * t1 + p2 * t2 + sqrt(max(0.0f, 1.0f - p1 * p1 - p2 * p2)) * vh;
    float3 h = safe_normalize(float3(alpha * nh.x, alpha * nh.y, max(0.0f, nh.z)), float3(0, 0, 1));
    float3 world_h = t * h.x + b * h.y + n * h.z;
    return reflect(-wo, world_h);
}
BsdfSample sample_bsdf(Surface s, float3 wo, thread uint &rng) {
    BsdfSample sample;
    float choose = random_float(rng);
    float2 uv = random_pair(rng);
    if (s.glass) {
        float3 reflected = sample_visible_ggx(wo, s.normal, s.roughness * s.roughness, uv);
        float3 h = safe_normalize(wo + reflected, s.normal);
        float eta = s.front_face ? s.ior : 1.0f / s.ior;
        float F = dielectric_fresnel(dot(wo, h), eta);
        sample.direction = choose < F ? reflected : refract(-wo, h, 1.0f / eta);
        sample.value = evaluate_glass(s, wo, sample.direction, sample.pdf);
        return sample;
    }
    sample.direction = choose < specular_probability(s, wo)
        ? sample_visible_ggx(wo, s.normal, s.roughness * s.roughness, uv)
        : cosine_sample(uv, s.normal);
    sample.value = evaluate_bsdf(s, wo, sample.direction, sample.pdf);
    return sample;
}

float3 environment(float3 direction, constant Uniforms &u) {
    return max(u.world.xyz, 0.0f) * max(u.world.w, 0.0f);
}
float environment_pdf(float3 direction, float3 normal, constant Uniforms &u) {
    return max(0.0f, dot(normal, direction)) * INV_PI_F;
}
float3 sample_environment(float3 normal, constant Uniforms &u, thread uint &rng) {
    // Preserve the established Rendered sample sequence while the studio
    // environment lives exclusively in the separate preview IBL pipeline.
    random_float(rng);
    return cosine_sample(random_pair(rng), normal);
}
float triangle_area(device const Triangle &tri) {
    return 0.5f * length(cross(tri.v1.xyz - tri.v0.xyz, tri.v2.xyz - tri.v0.xyz));
}
float area_light_pdf(device const Triangle &light, float3 from, float3 to, uint count) {
    float3 displacement = to - from;
    float distance_squared = dot(displacement, displacement);
    float area = triangle_area(light);
    if (area <= 1e-16f || distance_squared <= 1e-16f || count == 0u) return 0.0f;
    float3 light_normal = safe_normalize(cross(light.v1.xyz - light.v0.xyz, light.v2.xyz - light.v0.xyz), float3(0, 1, 0));
    float cosine = abs(dot(light_normal, displacement * rsqrt(distance_squared)));
    return cosine > 1e-8f ? distance_squared / (float(count) * area * cosine) : 0.0f;
}
float3 direct_lighting(Surface s, float3 wo, device const Triangle *triangles,
                       device const BvhNode *nodes, device const uint *lights,
                       constant Uniforms &u, thread uint &rng, MATERIAL_ARGS) {
    float3 result(0);
    if (u.scene.z > 0u) {
        uint light_index = lights[min(u.scene.z - 1u, uint(random_float(rng) * float(u.scene.z)))];
        if (light_index < u.scene.x) {
            device const Triangle &light = triangles[light_index];
            float2 uv = random_pair(rng); float r = sqrt(uv.x);
            float3 target = light.v0.xyz * (1.0f - r) + light.v1.xyz * (r * (1.0f - uv.y)) + light.v2.xyz * (r * uv.y);
            float3 delta = target - s.position;
            float distance_squared = dot(delta, delta);
            if (distance_squared > 1e-12f) {
                float3 wi = delta * rsqrt(distance_squared);
                float light_pdf = area_light_pdf(light, s.position, target, u.scene.z);
                float bsdf_pdf; float3 f = evaluate_bsdf(s, wo, wi, bsdf_pdf);
                if (light_pdf > 0.0f && bsdf_pdf > 0.0f) {
                    float3 origin = offset_origin(s.position, s.geometric_normal, wi);
                    float3 shadow_delta = target - origin;
                    float distance = length(shadow_delta);
                    Ray shadow = { origin, shadow_delta / distance };
                    if (trace(shadow, distance * (1.0f - 1e-5f), triangles, nodes, u, true).triangle == NO_HIT) {
                        Hit light_hit = { distance, float2(r * (1.0f - uv.y), r * uv.y), light_index };
                        float3 emission = evaluate_surface(shadow, light_hit, triangles, u, MATERIAL_PASS).emission;
                        result += emission * f * abs(dot(s.normal, wi))
                            * (power_heuristic(light_pdf, bsdf_pdf) / light_pdf);
                    }
                }
            }
        }
    }
    // Environment and finite emitters are separate estimators. Their sampled
    // directions do not double-count: environment samples contribute only if
    // unoccluded; finite-emitter samples terminate at their sampled emitter.
    float3 wi = sample_environment(s.normal, u, rng);
    float light_pdf = environment_pdf(wi, s.normal, u);
    float bsdf_pdf; float3 f = evaluate_bsdf(s, wo, wi, bsdf_pdf);
    if (light_pdf > 0.0f && bsdf_pdf > 0.0f) {
        Ray shadow = { offset_origin(s.position, s.geometric_normal, wi), wi };
        if (trace(shadow, INFINITY, triangles, nodes, u, true).triangle == NO_HIT)
            result += environment(wi, u) * f * abs(dot(s.normal, wi))
                * (power_heuristic(light_pdf, bsdf_pdf) / light_pdf);
    }
    return result;
}
float3 path_trace(Ray ray, Hit primary, device const Triangle *triangles, device const BvhNode *nodes,
                  device const uint *lights, constant Uniforms &u, thread uint &rng, MATERIAL_ARGS) {
    float3 radiance(0), throughput(1), previous_position(0), previous_normal(0);
    float previous_pdf = 0.0f;
    uint max_bounces = clamp(u.scene.w, 1u, 32u);
    for (uint bounce = 0u; bounce < max_bounces; ++bounce) {
        Hit hit = bounce == 0u ? primary : trace(ray, INFINITY, triangles, nodes, u);
        if (hit.triangle == NO_HIT) {
            float weight = bounce == 0u ? 1.0f : power_heuristic(previous_pdf, environment_pdf(ray.direction, previous_normal, u));
            radiance += throughput * environment(ray.direction, u) * weight;
            break;
        }
        Surface s = evaluate_surface(ray, hit, triangles, u, MATERIAL_PASS);
        if (max_component(s.emission) > 0.0f) {
            float light_pdf = bounce == 0u ? 0.0f : area_light_pdf(triangles[hit.triangle], previous_position, s.position, u.scene.z);
            float weight = bounce == 0u ? 1.0f : power_heuristic(previous_pdf, light_pdf);
            radiance += throughput * s.emission * weight;
        }
        float3 wo = -ray.direction;
        // At the terminal vertex no BSDF continuation exists, so NEE must use
        // weight 1 instead of a MIS weight whose counterpart was truncated.
        // A one-step terminal continuation below preserves the same estimator
        // and lets max_bounces bound scattering events rather than emission.
        radiance += throughput * direct_lighting(s, wo, triangles, nodes, lights, u, rng, MATERIAL_PASS);
        BsdfSample next = sample_bsdf(s, wo, rng);
        if (!(next.pdf > 1e-12f) || max_component(next.value) <= 0.0f) break;
        throughput *= next.value * (abs(dot(s.normal, next.direction)) / next.pdf);
        if (!all(isfinite(throughput)) || max_component(throughput) <= 0.0f) break;
        previous_position = s.position; previous_normal = s.normal; previous_pdf = next.pdf;
        ray = { offset_origin(s.position, s.geometric_normal, next.direction), next.direction };
        if (bounce + 1u == max_bounces) {
            Hit terminal = trace(ray, INFINITY, triangles, nodes, u);
            if (terminal.triangle == NO_HIT) {
                radiance += throughput * environment(ray.direction, u)
                    * power_heuristic(previous_pdf, environment_pdf(ray.direction, previous_normal, u));
            } else {
                Surface emitter = evaluate_surface(ray, terminal, triangles, u, MATERIAL_PASS);
                float light_pdf = area_light_pdf(triangles[terminal.triangle], previous_position, emitter.position, u.scene.z);
                radiance += throughput * emitter.emission * power_heuristic(previous_pdf, light_pdf);
            }
            break;
        }
        // Survival depends on throughput and is compensated exactly. No sample
        // clamping is applied: high-energy paths retain their expected energy.
        if (bounce >= 3u) {
            float survive = clamp(max_component(throughput), 0.05f, 0.95f);
            if (random_float(rng) >= survive) break;
            throughput /= survive;
        }
    }
    return all(isfinite(radiance)) ? max(radiance, 0.0f) : float3(0);
}

Ray camera_ray(float2 pixel, constant Uniforms &u) {
    float2 ndc = float2(2.0f * pixel.x / float(u.image.x) - 1.0f, 1.0f - 2.0f * pixel.y / float(u.image.y));
    if (u.camera.w > 0.5f) {
        float3 origin = u.cam_origin.xyz + u.cam_right.xyz * (ndc.x * u.camera.y * u.camera.z) + u.cam_up.xyz * (ndc.y * u.camera.z);
        return { origin, normalize(u.cam_forward.xyz) };
    }
    float3 direction = u.cam_forward.xyz + u.cam_right.xyz * (ndc.x * u.camera.y * u.camera.x) + u.cam_up.xyz * (ndc.y * u.camera.x);
    return { u.cam_origin.xyz, normalize(direction) };
}
float3 viewport_background(Ray ray) {
    float t = smoothstep(-0.5f, 0.7f, ray.direction.y);
    return mix(float3(0.029f, 0.034f, 0.043f), float3(0.018f, 0.023f, 0.032f), t);
}
float2 project_to_pixels(float3 point, constant Uniforms &u) {
    float3 delta = point - u.cam_origin.xyz;
    float half_height = u.camera.w > 0.5f ? u.camera.z : u.camera.x * max(1e-5f, dot(delta, u.cam_forward.xyz));
    return float2(dot(delta, u.cam_right.xyz), dot(delta, u.cam_up.xyz))
        * float2(float(u.image.x) / u.camera.y, float(u.image.y)) / (2.0f * half_height);
}
float projected_edge_distance(float3 position, float3 a, float3 b, constant Uniforms &u) {
    if (u.camera.w < 0.5f) {
        // Clip segments at the camera plane before perspective division. This
        // also handles being very close to or inside large imported objects.
        float da = dot(a - u.cam_origin.xyz, u.cam_forward.xyz);
        float db = dot(b - u.cam_origin.xyz, u.cam_forward.xyz);
        const float near_plane = 1e-4f;
        if (da < near_plane && db < near_plane) return INFINITY;
        if (da < near_plane) a = mix(a, b, (near_plane - da) / (db - da));
        else if (db < near_plane) b = mix(b, a, (near_plane - db) / (da - db));
    }
    float2 pa = project_to_pixels(a, u), pb = project_to_pixels(b, u);
    float2 point = project_to_pixels(position, u), edge = pb - pa;
    float t = clamp(dot(point - pa, edge) / max(1e-12f, dot(edge, edge)), 0.0f, 1.0f);
    return length(point - pa - t * edge);
}
float edge_coverage(Ray ray, Hit hit, device const Triangle &tri, constant Uniforms &u, float thickness) {
    float3 position = ray.origin + hit.distance * ray.direction;
    uint mask = uint(tri.params.w + 0.5f);
    float distance = INFINITY;
    if ((mask & 1u) != 0u) distance = min(distance, projected_edge_distance(position, tri.v1.xyz, tri.v2.xyz, u));
    if ((mask & 2u) != 0u) distance = min(distance, projected_edge_distance(position, tri.v2.xyz, tri.v0.xyz, u));
    if ((mask & 4u) != 0u) distance = min(distance, projected_edge_distance(position, tri.v0.xyz, tri.v1.xyz, u));
    return 1.0f - smoothstep(thickness * 0.45f, thickness * 1.45f, distance);
}
bool is_selected(Hit hit, device const Triangle *triangles, constant Uniforms &u) {
    return hit.triangle != NO_HIT && u.settings.z > 0.0f
        && abs(triangles[hit.triangle].params.z - u.settings.z) < 0.25f;
}
float selection_silhouette(float2 pixel, device const Triangle *triangles,
                           device const BvhNode *nodes, constant Uniforms &u) {
    // Called only on visible selected pixels: a one-pixel inward outline cannot
    // reveal hidden geometry. Comparing object IDs avoids tessellation edges,
    // while retaining silhouettes around holes and occlusion boundaries.
    const float2 offsets[4] = { float2(-1, 0), float2(1, 0), float2(0, -1), float2(0, 1) };
    for (uint i = 0u; i < 4u; ++i) {
        Hit neighbor = trace(camera_ray(pixel + offsets[i], u), INFINITY, triangles, nodes, u);
        if (!is_selected(neighbor, triangles, u)) return 1.0f;
    }
    return 0.0f;
}
float3 solid_shading(Surface s, Ray ray, constant Uniforms &u) {
    float3 key = normalize(-u.cam_forward.xyz * 0.6f - u.cam_right.xyz * 0.45f + u.cam_up.xyz * 0.85f);
    float3 fill = normalize(u.cam_right.xyz * 0.8f + u.cam_up.xyz * 0.2f - u.cam_forward.xyz * 0.25f);
    float3 rim = normalize(u.cam_forward.xyz * 0.8f + u.cam_up.xyz * 0.6f);
    float ambient = mix(0.15f, 0.26f, s.normal.y * 0.5f + 0.5f);
    float shade = ambient + 0.65f * max(0.0f, dot(s.normal, key)) + 0.17f * max(0.0f, dot(s.normal, fill));
    float specular = pow(max(0.0f, dot(s.normal, normalize(key - ray.direction))), 48.0f) * 0.12f;
    float rim_light = pow(max(0.0f, dot(s.normal, rim)), 3.0f) * 0.07f;
    return float3(0.39f, 0.43f, 0.49f) * shade + float3(specular + rim_light);
}
float grid_line(float coordinate, float spacing, float width) {
    float nearest = abs(coordinate - round(coordinate / spacing) * spacing);
    return 1.0f - smoothstep(width * 0.45f, width * 1.35f, nearest);
}
float3 add_grid(float3 color, Ray ray, Hit hit, float2 pixel, constant Uniforms &u) {
    if (u.settings.y < 0.5f || abs(ray.direction.y) < 1e-5f) return color;
    float distance = -ray.origin.y / ray.direction.y;
    if (distance <= 0.0f || distance > 10000.0f) return color;
    float forward_depth = distance * max(0.01f, dot(ray.direction, u.cam_forward.xyz));
    float pixel_world = 2.0f * (u.camera.w > 0.5f ? u.camera.z : forward_depth * u.camera.x) / float(u.image.y);
    // A viewport grid belongs behind coplanar scene surfaces. Account for both
    // floating-point scene scale and a small subpixel footprint, including the
    // larger ray-distance uncertainty when viewing the grid at grazing angles.
    float depth_tolerance = max(2e-5f * max(1.0f, distance),
        pixel_world * 0.05f / max(0.1f, abs(ray.direction.y)));
    if (distance + depth_tolerance >= hit.distance) return color;
    float3 p = ray.origin + distance * ray.direction;
    Ray rx = camera_ray(pixel + float2(1, 0), u), ry = camera_ray(pixel + float2(0, 1), u);
    if (abs(rx.direction.y) < 1e-5f || abs(ry.direction.y) < 1e-5f) return color;
    float3 px = rx.origin - rx.direction * (rx.origin.y / rx.direction.y);
    float3 py = ry.origin - ry.direction * (ry.origin.y / ry.direction.y);
    float wx = max(1e-6f, length(float2(px.x - p.x, py.x - p.x)));
    float wz = max(1e-6f, length(float2(px.z - p.z, py.z - p.z)));
    float footprint = max(wx, wz);
    float spacing = pow(10.0f, floor(log10(max(0.001f, footprint * 35.0f))));
    float minor = max(grid_line(p.x, spacing, wx), grid_line(p.z, spacing, wz));
    float major = max(grid_line(p.x, spacing * 10.0f, wx), grid_line(p.z, spacing * 10.0f, wz));
    float fade = (1.0f / (1.0f + distance * 0.025f)) * smoothstep(0.015f, 0.12f, abs(ray.direction.y));
    float opacity = (0.11f * minor + 0.13f * major) * fade;
    color = mix(color, float3(0.13f, 0.15f, 0.18f), opacity);
    float x_axis = 1.0f - smoothstep(wz * 0.5f, wz * 1.6f, abs(p.z));
    float z_axis = 1.0f - smoothstep(wx * 0.5f, wx * 1.6f, abs(p.x));
    color = mix(color, float3(0.23f, 0.055f, 0.06f), x_axis * fade * 0.55f);
    return mix(color, float3(0.055f, 0.12f, 0.29f), z_axis * fade * 0.55f);
}
float3 display_transform(float3 linear, float exposure) {
    float3 x = max(linear, 0.0f) * exp2(clamp(exposure, -20.0f, 20.0f));
    // Fitted filmic display curve (ACES approximation), followed by sRGB OETF.
    float3 mapped = clamp((x * (2.51f * x + 0.03f)) / (x * (2.43f * x + 0.59f) + 0.14f), 0.0f, 1.0f);
    return select(1.055f * pow(mapped, float3(1.0f / 2.4f)) - 0.055f, 12.92f * mapped, mapped <= 0.0031308f);
}

float3 wireframe_shading(Ray ray, float2 pixel, Hit primary, device const Triangle *triangles,
                          device const BvhNode *nodes, constant Uniforms &u) {
    // Blender-style X-ray wireframe: faces are transparent, so every polygon
    // edge along the ray stays visible. Front edges are opaque and bright;
    // occluded edges behind them are dimmed rather than removed.
    float3 color = viewport_background(ray) * 1.10f;
    Hit far_hit = { INFINITY, float2(0), NO_HIT };
    color = add_grid(color, ray, far_hit, pixel, u);
    Ray march = ray;
    Hit hit = primary;
    // Entry + exit of one closed mesh needs two layers; four covers two
    // overlapping objects while bounding per-pixel traversal cost.
    for (uint layer = 0u; layer < 4u; ++layer) {
        if (layer > 0u) {
            hit = trace(march, INFINITY, triangles, nodes, u);
        }
        if (hit.triangle == NO_HIT) break;
        device const Triangle &tri = triangles[hit.triangle];
        float edge = edge_coverage(march, hit, tri, u, 1.0f);
        if (edge > 0.0f) {
            bool selected = is_selected(hit, triangles, u);
            float3 line = selected
                ? float3(0.055f, 0.40f, 0.95f)
                : float3(0.82f, 0.86f, 0.92f);
            float alpha = layer == 0u ? edge : edge * 0.35f;
            if (selected) alpha = layer == 0u ? edge * 0.92f : edge * 0.45f;
            color = mix(color, line, clamp(alpha, 0.0f, 1.0f));
        }
        float3 position = march.origin + march.direction * hit.distance;
        float step = max(1e-4f, hit.distance * 1e-4f);
        march.origin = position + ray.direction * step;
    }
    return color;
}

kernel void render_main(device const Triangle *triangles [[buffer(0)]],
                        device const BvhNode *nodes [[buffer(1)]],
                        constant Uniforms &u [[buffer(2)]],
                        device float4 *accumulation [[buffer(3)]],
                        device const uint *lights [[buffer(4)]],
                        device const GpuMaterial *materials [[buffer(6)]],
                        device const float4 *texture_pixels [[buffer(7)]],
                        device const uint4 *texture_levels [[buffer(8)]],
                        texture2d<float, access::write> output [[texture(0)]],
                        uint2 gid [[thread_position_in_grid]]) {
    if (gid.x >= u.image.x || gid.y >= u.image.y) return;
    uint index = gid.y * u.image.x + gid.x;
    uint rng = hash_u32(index ^ hash_u32(u.image.z + 0x9e3779b9u));
    bool progressive = u.image.w == 3u;
    float2 pixel = float2(gid) + (progressive ? random_pair(rng) : float2(0.5f));
    Ray ray = camera_ray(pixel, u);
    Hit primary = trace(ray, INFINITY, triangles, nodes, u);
    float3 color;
    if (progressive) {
        color = path_trace(ray, primary, triangles, nodes, lights, u, rng, MATERIAL_PASS);
    } else if (u.image.w == 0u) {
        color = wireframe_shading(ray, pixel, primary, triangles, nodes, u);
    } else if (primary.triangle != NO_HIT) {
        Surface s = surface_at(ray, primary, triangles);
        color = solid_shading(s, ray, u);
    } else color = viewport_background(ray);
    if (u.image.w == 1u) {
        color = add_grid(color, ray, primary, pixel, u);
        if (is_selected(primary, triangles, u)) {
            float edge = selection_silhouette(pixel, triangles, nodes, u);
            color = mix(color, float3(0.055f, 0.40f, 0.95f), edge * 0.92f);
        }
    }
    if (progressive) {
        float4 total = u.image.z == 0u ? float4(color, 1.0f) : accumulation[index] + float4(color, 1.0f);
        accumulation[index] = total;
        color = total.xyz / max(1.0f, total.w);
    } else accumulation[index] = float4(color, 1.0f);
    output.write(float4(display_transform(color, progressive ? u.settings.x : 0.0f), 1.0f), gid);
}
