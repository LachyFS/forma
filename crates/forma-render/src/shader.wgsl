// Portable counterpart of shader.metal. Keep the scene-linear shading model,
// sample sequence, original polygon edges and display transform in sync.
struct Triangle {
    v0: vec4<f32>, v1: vec4<f32>, v2: vec4<f32>,
    n0: vec4<f32>, n1: vec4<f32>, n2: vec4<f32>,
    base_color: vec4<f32>, emission: vec4<f32>, params: vec4<f32>,
    generated0: vec4<f32>, generated1: vec4<f32>, generated2: vec4<f32>,
}
struct BvhNode { minimum: vec4<f32>, maximum: vec4<f32>, data: vec4<u32> }
struct Uniforms {
    cam_origin: vec4<f32>, cam_right: vec4<f32>, cam_up: vec4<f32>, cam_forward: vec4<f32>,
    world: vec4<f32>, camera: vec4<f32>, image: vec4<u32>, scene: vec4<u32>,
    settings: vec4<f32>, preview_lighting: vec4<f32>, preview_display: vec4<f32>,
}
struct Ray { origin: vec3<f32>, direction: vec3<f32> }
struct Hit { distance: f32, bary: vec2<f32>, triangle: u32 }
struct Surface {
    position: vec3<f32>, normal: vec3<f32>, geometric_normal: vec3<f32>,
    color: vec3<f32>, emission: vec3<f32>, metallic: f32, roughness: f32,
    ior: f32, front_face: bool, glass: bool,
}
struct BsdfSample { direction: vec3<f32>, value: vec3<f32>, pdf: f32 }
struct BsdfValue { value: vec3<f32>, pdf: f32 }
struct Basis { tangent: vec3<f32>, bitangent: vec3<f32> }

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> triangles: array<Triangle>;
@group(0) @binding(2) var<storage, read> nodes: array<BvhNode>;
@group(0) @binding(3) var<storage, read> lights: array<u32>;
@group(0) @binding(4) var<storage, read_write> accumulation: array<vec4<f32>>;
@group(0) @binding(5) var output: texture_storage_2d<rgba8unorm, write>;

const PI_F: f32 = 3.14159265358979323846;
const INV_PI_F: f32 = 0.31830988618379067154;
const NO_HIT: u32 = 0xffffffffu;
// WGSL deliberately excludes infinity literals. This finite sentinel also
// makes BVH traversal independent of the backend's infinity optimizations.
const FAR: f32 = 3.402823466e38;

fn hash_u32(input: u32) -> u32 {
    var x = input;
    x ^= x >> 16u; x *= 0x7feb352du;
    x ^= x >> 15u; x *= 0x846ca68bu;
    return x ^ (x >> 16u);
}
fn random_float(state: ptr<function, u32>) -> f32 {
    *state = *state * 747796405u + 2891336453u;
    let word = ((*state >> ((*state >> 28u) + 4u)) ^ *state) * 277803737u;
    return f32(((word >> 22u) ^ word) >> 8u) * (1.0 / 16777216.0);
}
fn random_pair(state: ptr<function, u32>) -> vec2<f32> {
    let x = random_float(state);
    return vec2(x, random_float(state));
}
fn luminance(v: vec3<f32>) -> f32 { return dot(v, vec3(0.2126, 0.7152, 0.0722)); }
fn max_component(v: vec3<f32>) -> f32 { return max(v.x, max(v.y, v.z)); }
fn finite3(v: vec3<f32>) -> bool { return all(abs(v) <= vec3(FAR)); }
fn safe_normalize(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let len2 = dot(v, v);
    if len2 > 1e-20 { return v * inverseSqrt(len2); }
    return fallback;
}
fn basis(n: vec3<f32>) -> Basis {
    let axis = select(vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), abs(n.y) < 0.999);
    let t = safe_normalize(cross(axis, n), vec3(1.0, 0.0, 0.0));
    return Basis(t, cross(n, t));
}
fn local_to_world(v: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let frame = basis(n);
    return frame.tangent * v.x + frame.bitangent * v.y + n * v.z;
}
fn cosine_sample(uv: vec2<f32>, n: vec3<f32>) -> vec3<f32> {
    let r = sqrt(uv.x); let phi = 2.0 * PI_F * uv.y;
    return local_to_world(vec3(r * cos(phi), r * sin(phi), sqrt(max(0.0, 1.0 - uv.x))), n);
}
fn power_heuristic(a: f32, b: f32) -> f32 {
    if a <= 0.0 { return 0.0; }
    let ratio = b / a;
    return 1.0 / (1.0 + ratio * ratio);
}
fn offset_component(p: f32, n: f32) -> f32 {
    let offset = i32(256.0 * n);
    let shifted = bitcast<f32>(bitcast<i32>(p) + select(offset, -offset, p < 0.0));
    return select(shifted, p + n * (1.0 / 65536.0), abs(p) < (1.0 / 32.0));
}
fn offset_origin(p: vec3<f32>, normal: vec3<f32>, outgoing: vec3<f32>) -> vec3<f32> {
    let n = select(-normal, normal, dot(normal, outgoing) >= 0.0);
    return vec3(offset_component(p.x, n.x), offset_component(p.y, n.y), offset_component(p.z, n.z));
}
fn inverse_component(v: f32) -> f32 {
    if abs(v) < 1e-30 { return select(1e30, -1e30, v < 0.0); }
    return 1.0 / v;
}
fn box_distance(ray: Ray, inverse_dir: vec3<f32>, node: BvhNode, limit: f32) -> f32 {
    let a = (node.minimum.xyz - ray.origin) * inverse_dir;
    let b = (node.maximum.xyz - ray.origin) * inverse_dir;
    let lo = min(a, b); let hi = max(a, b);
    let near_t = max(0.0, max_component(lo));
    let far_t = min(hi.x, min(hi.y, hi.z));
    return select(FAR, near_t, far_t >= near_t && near_t < limit);
}
fn triangle_hit(ray: Ray, tri: Triangle, hit: ptr<function, Hit>, index: u32) -> bool {
    let e1 = tri.v1.xyz - tri.v0.xyz; let e2 = tri.v2.xyz - tri.v0.xyz;
    let p = cross(ray.direction, e2);
    let determinant = dot(e1, p);
    let scale = sqrt(dot(e1, e1) * dot(e2, e2));
    if abs(determinant) <= max(1e-30, scale * 1e-8) { return false; }
    let inv = 1.0 / determinant;
    let t = ray.origin - tri.v0.xyz;
    let bary_u = dot(t, p) * inv;
    if bary_u < 0.0 || bary_u > 1.0 { return false; }
    let q = cross(t, e1);
    let bary_v = dot(ray.direction, q) * inv;
    if bary_v < 0.0 || bary_u + bary_v > 1.0 { return false; }
    let distance = dot(e2, q) * inv;
    if distance <= 1e-7 || distance >= (*hit).distance { return false; }
    *hit = Hit(distance, vec2(bary_u, bary_v), index);
    return true;
}
fn trace(ray: Ray, limit: f32, any_hit: bool) -> Hit {
    var hit = Hit(limit, vec2(0.0), NO_HIT);
    if u.scene.x == 0u || u.scene.y == 0u { return hit; }
    let inverse_dir = vec3(inverse_component(ray.direction.x), inverse_component(ray.direction.y), inverse_component(ray.direction.z));
    var stack: array<u32, 64>;
    var stack_size = 1u; stack[0] = 0u;
    var overflow = false;
    while stack_size > 0u {
        stack_size -= 1u;
        let index = stack[stack_size];
        if index >= u.scene.y { continue; }
        let node = nodes[index];
        if box_distance(ray, inverse_dir, node, hit.distance) == FAR { continue; }
        if node.data.z != 0u {
            let end = min(u.scene.x, node.data.x + node.data.y);
            for (var i = node.data.x; i < end; i += 1u) {
                if triangle_hit(ray, triangles[i], &hit, i) && any_hit { return hit; }
            }
        } else {
            let left = node.data.x; let right = node.data.y;
            var dl = FAR; var dr = FAR;
            if left < u.scene.y { dl = box_distance(ray, inverse_dir, nodes[left], hit.distance); }
            if right < u.scene.y { dr = box_distance(ray, inverse_dir, nodes[right], hit.distance); }
            let near_node = select(right, left, dl < dr); let far_node = select(left, right, dl < dr);
            let near_dist = min(dl, dr); let far_dist = max(dl, dr);
            let needed = u32(near_dist != FAR) + u32(far_dist != FAR);
            if stack_size + needed > 64u { overflow = true; break; }
            if far_dist != FAR { stack[stack_size] = far_node; stack_size += 1u; }
            if near_dist != FAR { stack[stack_size] = near_node; stack_size += 1u; }
        }
    }
    if overflow {
        for (var i = 0u; i < u.scene.x; i += 1u) {
            if triangle_hit(ray, triangles[i], &hit, i) && any_hit { break; }
        }
    }
    return hit;
}
fn surface_at(ray: Ray, hit: Hit) -> Surface {
    let tri = triangles[hit.triangle];
    let weights = vec3(1.0 - hit.bary.x - hit.bary.y, hit.bary.x, hit.bary.y);
    var geometric = safe_normalize(cross(tri.v1.xyz - tri.v0.xyz, tri.v2.xyz - tri.v0.xyz), vec3(0.0, 1.0, 0.0));
    var normal = safe_normalize(weights.x * tri.n0.xyz + weights.y * tri.n1.xyz + weights.z * tri.n2.xyz, geometric);
    if dot(normal, geometric) < 0.0 { geometric = -geometric; }
    let front_face = dot(geometric, ray.direction) < 0.0;
    if dot(geometric, ray.direction) > 0.0 { geometric = -geometric; normal = -normal; }
    if dot(normal, -ray.direction) < 1e-4 { normal = geometric; }
    return Surface(ray.origin + ray.direction * hit.distance, normal, geometric,
        clamp(tri.base_color.xyz, vec3(0.0), vec3(1.0)), max(tri.emission.xyz, vec3(0.0)),
        clamp(tri.params.x, 0.0, 1.0), clamp(tri.params.y, 0.025, 1.0), 1.5, front_face, false);
}
struct GpuMaterial {
    color: vec4<f32>, emission: vec4<f32>, params: vec4<f32>, mapping: vec4<f32>,
    info: vec4<u32>, textures: array<u32, 8>,
}
struct ShaderInput {
    uv: vec2<f32>, generated: vec3<f32>, position: vec3<f32>, normal: vec3<f32>, view_direction: vec3<f32>,
}
@group(0) @binding(6) var<storage, read> materials: array<GpuMaterial>;
@group(0) @binding(7) var<storage, read> texture_pixels: array<vec4<f32>>;
@group(0) @binding(8) var<storage, read> texture_levels: array<vec4<u32>>;

// FORMA_CUSTOM_FUNCTIONS
fn forma_custom(index: u32, surface: Surface, input: ShaderInput) -> Surface { return surface; }

fn generated_uv(p: vec3<f32>, face_normal: vec3<f32>, mapping: u32) -> vec2<f32> {
    if mapping == 1u {
        let d = safe_normalize(p - vec3(0.5), vec3(0.0, 1.0, 0.0));
        return vec2(atan2(d.z, d.x) / (2.0 * PI_F) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI_F);
    }
    if mapping == 2u { return vec2(p.x, 1.0 - p.z); }
    let a = abs(face_normal);
    if a.x >= a.y && a.x >= a.z { return vec2(select(p.z, 1.0 - p.z, face_normal.x >= 0.0), 1.0 - p.y); }
    if a.y >= a.z { return vec2(p.x, select(p.z, 1.0 - p.z, face_normal.y >= 0.0)); }
    return vec2(select(1.0 - p.x, p.x, face_normal.z >= 0.0), 1.0 - p.y);
}
fn wrap_texel(p: i32, size: u32) -> u32 { let n = i32(size); return u32((p % n + n) % n); }
fn bilinear_texture(desc: vec4<u32>, uv: vec2<f32>) -> vec4<f32> {
    let p = fract(uv) * vec2<f32>(desc.yz) - vec2(0.5);
    let a = vec2<i32>(floor(p)); let f = fract(p);
    let x0 = wrap_texel(a.x, desc.y); let x1 = wrap_texel(a.x + 1, desc.y);
    let y0 = wrap_texel(a.y, desc.z); let y1 = wrap_texel(a.y + 1, desc.z);
    return mix(mix(texture_pixels[desc.x + y0 * desc.y + x0], texture_pixels[desc.x + y0 * desc.y + x1], f.x),
               mix(texture_pixels[desc.x + y1 * desc.y + x0], texture_pixels[desc.x + y1 * desc.y + x1], f.x), f.y);
}
fn image_texture(index: u32, uv: vec2<f32>, footprint: f32) -> vec4<f32> {
    if index == NO_HIT { return vec4(1.0); }
    let desc = texture_levels[index];
    let lod = clamp(log2(max(1.0, footprint * f32(max(desc.y, desc.z)))), 0.0, f32(desc.w - 1u));
    let lo = u32(floor(lod)); let hi = min(lo + 1u, desc.w - 1u);
    return mix(bilinear_texture(texture_levels[index + lo], uv), bilinear_texture(texture_levels[index + hi], uv), fract(lod));
}
fn evaluate_surface(ray: Ray, hit: Hit) -> Surface {
    var s = surface_at(ray, hit);
    let tri = triangles[hit.triangle]; let m = materials[u32(tri.generated0.w)];
    let weights = vec3(1.0 - hit.bary.x - hit.bary.y, hit.bary.x, hit.bary.y);
    let q0 = tri.generated0.xyz; let q1 = tri.generated1.xyz; let q2 = tri.generated2.xyz;
    let generated = q0 * weights.x + q1 * weights.y + q2 * weights.z;
    let face = safe_normalize(cross(q1 - q0, q2 - q0), vec3(0.0, 1.0, 0.0));
    let uv = generated_uv(generated, face, m.info.y) * m.mapping.xy + m.mapping.zw;
    let uv0 = generated_uv(q0, face, m.info.y);
    var uv1 = generated_uv(q1, face, m.info.y); var uv2 = generated_uv(q2, face, m.info.y);
    if m.info.y == 1u { uv1.x -= round(uv1.x - uv0.x); uv2.x -= round(uv2.x - uv0.x); }
    let d1 = (uv1 - uv0) * m.mapping.xy; let d2 = (uv2 - uv0) * m.mapping.xy;
    let e1 = tri.v1.xyz - tri.v0.xyz; let e2 = tri.v2.xyz - tri.v0.xyz;
    let density = max(length(d1) / max(length(e1), 1e-6), length(d2) / max(length(e2), 1e-6));
    let pixel_width = 2.0 * select(hit.distance * u.camera.x, u.camera.z, u.camera.w > 0.5) / f32(u.image.y);
    let footprint = density * pixel_width / max(0.15, abs(dot(s.normal, ray.direction)));
    s.color = m.color.xyz * image_texture(m.textures[0], uv, footprint).xyz;
    s.emission = m.emission.xyz * image_texture(m.textures[4], uv, footprint).xyz;
    s.metallic = m.params.x; s.roughness = m.params.y;
    if m.textures[2] != NO_HIT { s.metallic = image_texture(m.textures[2], uv, footprint).x; }
    if m.textures[1] != NO_HIT { s.roughness = image_texture(m.textures[1], uv, footprint).x; }
    s.ior = m.params.z; s.glass = m.info.x == 1u;
    let determinant = d1.x * d2.y - d1.y * d2.x;
    if m.textures[3] != NO_HIT && abs(determinant) > 1e-8 {
        var t = (e1 * d2.y - e2 * d1.y) / determinant;
        t = safe_normalize(t - s.normal * dot(t, s.normal), vec3(1.0, 0.0, 0.0));
        var b = cross(s.normal, t);
        let dpdv = (e2 * d1.x - e1 * d2.x) / determinant;
        if dot(b, dpdv) > 0.0 { b = -b; }
        var map = image_texture(m.textures[3], uv, footprint).xyz * 2.0 - vec3(1.0);
        map.x *= m.params.w; map.y *= m.params.w;
        s.normal = safe_normalize(t * map.x + b * map.y + s.normal * map.z, s.normal);
    }
    let before = s;
    if m.info.x == 2u { s = forma_custom(m.info.z, s, ShaderInput(uv, generated, s.position, s.normal, -ray.direction)); }
    // Protect geometry and film from malformed numerical output.
    s.position = before.position; s.geometric_normal = before.geometric_normal; s.front_face = before.front_face;
    s.color = select(before.color, clamp(s.color, vec3(0.0), vec3(1.0)), finite3(s.color));
    s.emission = select(before.emission, clamp(s.emission, vec3(0.0), vec3(1e6)), finite3(s.emission));
    s.metallic = select(m.params.x, clamp(s.metallic, 0.0, 1.0), abs(s.metallic) <= FAR);
    s.roughness = select(max(0.025, m.params.y), clamp(s.roughness, 0.025, 1.0), abs(s.roughness) <= FAR);
    s.ior = select(m.params.z, clamp(s.ior, 1.01, 3.0), abs(s.ior) <= FAR);
    s.normal = select(s.geometric_normal, safe_normalize(s.normal, s.geometric_normal), finite3(s.normal));
    if dot(s.normal, s.geometric_normal) < 1e-4 || dot(s.normal, -ray.direction) < 1e-4 { s.normal = s.geometric_normal; }
    return s;
}
fn dielectric_f0(ior: f32) -> f32 { let r = (ior - 1.0) / (ior + 1.0); return r * r; }
fn dielectric_fresnel(cosine: f32, eta: f32) -> f32 {
    let c = clamp(abs(cosine), 0.0, 1.0); let sin2_t = (1.0 - c * c) / (eta * eta);
    if sin2_t >= 1.0 { return 1.0; }
    let ct = sqrt(max(0.0, 1.0 - sin2_t));
    let rs = (c - eta * ct) / max(1e-12, c + eta * ct);
    let rp = (eta * c - ct) / max(1e-12, eta * c + ct);
    return 0.5 * (rs * rs + rp * rp);
}
fn evaluate_glass(s: Surface, wo: vec3<f32>, wi: vec3<f32>) -> BsdfValue {
    let nv = dot(s.normal, wo); let nl = dot(s.normal, wi);
    if nv <= 0.0 || abs(nl) < 1e-7 || nl * dot(s.geometric_normal, wi) <= 0.0 { return BsdfValue(vec3(0.0), 0.0); }
    let reflection = nl > 0.0; let eta = select(1.0 / s.ior, s.ior, s.front_face);
    var h = safe_normalize(wo + wi * select(eta, 1.0, reflection), s.normal);
    if dot(h, s.normal) < 0.0 { h = -h; }
    let vh = dot(wo, h); let lh = dot(wi, h);
    if vh <= 0.0 || lh * nl <= 0.0 { return BsdfValue(vec3(0.0), 0.0); }
    let alpha = s.roughness * s.roughness;
    let D = ggx_D(max(0.0, dot(s.normal, h)), alpha);
    let Gv = ggx_G1(nv, alpha); let Gl = ggx_G1(abs(nl), alpha);
    let F = dielectric_fresnel(vh, eta); let half_pdf = D * Gv * vh / nv;
    if reflection {
        return BsdfValue(vec3(F * D * Gv * Gl / max(1e-12, 4.0 * nv * nl)), half_pdf * F / (4.0 * vh));
    }
    let d = lh + vh / eta; let denom = max(1e-16, d * d);
    return BsdfValue(s.color * ((1.0 - F) * D * Gv * Gl * abs(lh * vh / (nl * nv)) / (denom * eta * eta)),
                     half_pdf * (1.0 - F) * abs(lh) / denom);
}

fn fresnel(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    let x = clamp(1.0 - cos_theta, 0.0, 1.0); let x2 = x * x;
    return f0 + (vec3(1.0) - f0) * (x2 * x2 * x);
}
fn ggx_D(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI_F * d * d);
}
fn ggx_G1(n_dot_v: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    return (2.0 * n_dot_v) / max(1e-12, n_dot_v + sqrt(a2 + (1.0 - a2) * n_dot_v * n_dot_v));
}
fn specular_probability(s: Surface, wo: vec3<f32>) -> f32 {
    let f0 = mix(vec3(dielectric_f0(s.ior)), s.color, s.metallic);
    let reflectance = luminance(fresnel(max(0.0, dot(s.normal, wo)), f0));
    let diffuse = luminance(s.color) * (1.0 - s.metallic);
    return select(clamp(reflectance / max(1e-6, reflectance + diffuse), 0.1, 0.95), 1.0, s.metallic > 0.999);
}
fn evaluate_bsdf(s: Surface, wo: vec3<f32>, wi: vec3<f32>) -> BsdfValue {
    if s.glass { return evaluate_glass(s, wo, wi); }
    let nv = dot(s.normal, wo); let nl = dot(s.normal, wi);
    if nv <= 0.0 || nl <= 0.0 || dot(s.geometric_normal, wi) <= 0.0 { return BsdfValue(vec3(0.0), 0.0); }
    let h = safe_normalize(wo + wi, s.normal);
    let nh = max(0.0, dot(s.normal, h)); let vh = max(0.0, dot(wo, h));
    let alpha = s.roughness * s.roughness;
    let D = ggx_D(nh, alpha); let Gv = ggx_G1(nv, alpha); let Gl = ggx_G1(nl, alpha);
    let F = fresnel(vh, mix(vec3(dielectric_f0(s.ior)), s.color, s.metallic));
    let specular = F * (D * Gv * Gl / max(1e-12, 4.0 * nv * nl));
    let diffuse = (vec3(1.0) - F) * (1.0 - s.metallic) * s.color * INV_PI_F;
    let p_spec = specular_probability(s, wo);
    let spec_pdf = D * Gv / max(1e-12, 4.0 * nv);
    return BsdfValue(diffuse + specular, mix(nl * INV_PI_F, spec_pdf, p_spec));
}
fn sample_visible_ggx(wo: vec3<f32>, n: vec3<f32>, alpha: f32, uv: vec2<f32>) -> vec3<f32> {
    let frame = basis(n); let t = frame.tangent; let b = frame.bitangent;
    let v = vec3(dot(wo, t), dot(wo, b), dot(wo, n));
    let vh = safe_normalize(vec3(alpha * v.x, alpha * v.y, v.z), vec3(0.0, 0.0, 1.0));
    let len2 = vh.x * vh.x + vh.y * vh.y;
    var t1 = vec3(1.0, 0.0, 0.0);
    if len2 > 0.0 { t1 = vec3(-vh.y, vh.x, 0.0) * inverseSqrt(len2); }
    let t2 = cross(vh, t1);
    let r = sqrt(uv.x); let phi = 2.0 * PI_F * uv.y;
    let p1 = r * cos(phi); var p2 = r * sin(phi);
    let blend = 0.5 * (1.0 + vh.z);
    p2 = (1.0 - blend) * sqrt(max(0.0, 1.0 - p1 * p1)) + blend * p2;
    let nh = p1 * t1 + p2 * t2 + sqrt(max(0.0, 1.0 - p1 * p1 - p2 * p2)) * vh;
    let h = safe_normalize(vec3(alpha * nh.x, alpha * nh.y, max(0.0, nh.z)), vec3(0.0, 0.0, 1.0));
    return reflect(-wo, t * h.x + b * h.y + n * h.z);
}
fn sample_bsdf(s: Surface, wo: vec3<f32>, rng: ptr<function, u32>) -> BsdfSample {
    let choose = random_float(rng); let uv = random_pair(rng);
    var direction: vec3<f32>;
    if s.glass {
        let reflected = sample_visible_ggx(wo, s.normal, s.roughness * s.roughness, uv);
        let h = safe_normalize(wo + reflected, s.normal);
        let eta = select(1.0 / s.ior, s.ior, s.front_face);
        let F = dielectric_fresnel(dot(wo, h), eta);
        direction = select(refract(-wo, h, 1.0 / eta), reflected, choose < F);
        let bsdf = evaluate_glass(s, wo, direction);
        return BsdfSample(direction, bsdf.value, bsdf.pdf);
    }
    if choose < specular_probability(s, wo) {
        direction = sample_visible_ggx(wo, s.normal, s.roughness * s.roughness, uv);
    } else { direction = cosine_sample(uv, s.normal); }
    let bsdf = evaluate_bsdf(s, wo, direction);
    return BsdfSample(direction, bsdf.value, bsdf.pdf);
}
fn environment(direction: vec3<f32>) -> vec3<f32> {
    return max(u.world.xyz, vec3(0.0)) * max(u.world.w, 0.0);
}
fn environment_pdf(direction: vec3<f32>, normal: vec3<f32>) -> f32 {
    return max(0.0, dot(normal, direction)) * INV_PI_F;
}
fn sample_environment(normal: vec3<f32>, rng: ptr<function, u32>) -> vec3<f32> {
    let unused = random_float(rng);
    return cosine_sample(random_pair(rng), normal);
}
fn triangle_area(tri: Triangle) -> f32 {
    return 0.5 * length(cross(tri.v1.xyz - tri.v0.xyz, tri.v2.xyz - tri.v0.xyz));
}
fn area_light_pdf(light: Triangle, source_position: vec3<f32>, target_position: vec3<f32>, count: u32) -> f32 {
    let displacement = target_position - source_position; let distance_squared = dot(displacement, displacement);
    let area = triangle_area(light);
    if area <= 1e-16 || distance_squared <= 1e-16 || count == 0u { return 0.0; }
    let light_normal = safe_normalize(cross(light.v1.xyz - light.v0.xyz, light.v2.xyz - light.v0.xyz), vec3(0.0, 1.0, 0.0));
    let cosine = abs(dot(light_normal, displacement * inverseSqrt(distance_squared)));
    if cosine > 1e-8 { return distance_squared / (f32(count) * area * cosine); }
    return 0.0;
}
fn direct_lighting(s: Surface, wo: vec3<f32>, rng: ptr<function, u32>) -> vec3<f32> {
    var result = vec3(0.0);
    if u.scene.z > 0u {
        let light_index = lights[min(u.scene.z - 1u, u32(random_float(rng) * f32(u.scene.z)))];
        if light_index < u.scene.x {
            let light = triangles[light_index];
            let uv = random_pair(rng); let r = sqrt(uv.x);
            let light_position = light.v0.xyz * (1.0 - r) + light.v1.xyz * (r * (1.0 - uv.y)) + light.v2.xyz * (r * uv.y);
            let delta = light_position - s.position; let distance_squared = dot(delta, delta);
            if distance_squared > 1e-12 {
                let wi = delta * inverseSqrt(distance_squared);
                let light_pdf = area_light_pdf(light, s.position, light_position, u.scene.z);
                let bsdf = evaluate_bsdf(s, wo, wi);
                if light_pdf > 0.0 && bsdf.pdf > 0.0 {
                    let origin = offset_origin(s.position, s.geometric_normal, wi);
                    let shadow_delta = light_position - origin; let distance = length(shadow_delta);
                    let shadow = Ray(origin, shadow_delta / distance);
                    if trace(shadow, distance * (1.0 - 1e-5), true).triangle == NO_HIT {
                        let light_hit = Hit(distance, vec2(r * (1.0 - uv.y), r * uv.y), light_index);
                        let emission = evaluate_surface(shadow, light_hit).emission;
                        result += emission * bsdf.value * abs(dot(s.normal, wi))
                            * (power_heuristic(light_pdf, bsdf.pdf) / light_pdf);
                    }
                }
            }
        }
    }
    let wi = sample_environment(s.normal, rng);
    let light_pdf = environment_pdf(wi, s.normal);
    let bsdf = evaluate_bsdf(s, wo, wi);
    if light_pdf > 0.0 && bsdf.pdf > 0.0 {
        let shadow = Ray(offset_origin(s.position, s.geometric_normal, wi), wi);
        if trace(shadow, FAR, true).triangle == NO_HIT {
            result += environment(wi) * bsdf.value * abs(dot(s.normal, wi))
                * (power_heuristic(light_pdf, bsdf.pdf) / light_pdf);
        }
    }
    return result;
}
fn path_trace(initial_ray: Ray, primary: Hit, rng: ptr<function, u32>) -> vec3<f32> {
    var ray = initial_ray;
    var radiance = vec3(0.0); var throughput = vec3(1.0);
    var previous_position = vec3(0.0); var previous_normal = vec3(0.0); var previous_pdf = 0.0;
    let max_bounces = clamp(u.scene.w, 1u, 32u);
    for (var bounce = 0u; bounce < max_bounces; bounce += 1u) {
        var hit = primary;
        if bounce > 0u { hit = trace(ray, FAR, false); }
        if hit.triangle == NO_HIT {
            var weight = 1.0;
            if bounce > 0u { weight = power_heuristic(previous_pdf, environment_pdf(ray.direction, previous_normal)); }
            radiance += throughput * environment(ray.direction) * weight;
            break;
        }
        let s = evaluate_surface(ray, hit);
        if max_component(s.emission) > 0.0 {
            var weight = 1.0;
            if bounce > 0u {
                let light_pdf = area_light_pdf(triangles[hit.triangle], previous_position, s.position, u.scene.z);
                weight = power_heuristic(previous_pdf, light_pdf);
            }
            radiance += throughput * s.emission * weight;
        }
        let wo = -ray.direction;
        radiance += throughput * direct_lighting(s, wo, rng);
        let next = sample_bsdf(s, wo, rng);
        if !(next.pdf > 1e-12) || max_component(next.value) <= 0.0 { break; }
        throughput *= next.value * (abs(dot(s.normal, next.direction)) / next.pdf);
        if !finite3(throughput) || max_component(throughput) <= 0.0 { break; }
        previous_position = s.position; previous_normal = s.normal; previous_pdf = next.pdf;
        ray = Ray(offset_origin(s.position, s.geometric_normal, next.direction), next.direction);
        // Include the complementary terminal emission path to preserve MIS energy.
        if bounce + 1u == max_bounces {
            let terminal = trace(ray, FAR, false);
            if terminal.triangle == NO_HIT {
                radiance += throughput * environment(ray.direction)
                    * power_heuristic(previous_pdf, environment_pdf(ray.direction, previous_normal));
            } else {
                let emitter = evaluate_surface(ray, terminal);
                let light_pdf = area_light_pdf(triangles[terminal.triangle], previous_position, emitter.position, u.scene.z);
                radiance += throughput * emitter.emission * power_heuristic(previous_pdf, light_pdf);
            }
            break;
        }
        if bounce >= 3u {
            let survive = clamp(max_component(throughput), 0.05, 0.95);
            if random_float(rng) >= survive { break; }
            throughput /= survive;
        }
    }
    if finite3(radiance) { return max(radiance, vec3(0.0)); }
    return vec3(0.0);
}
fn camera_ray(pixel: vec2<f32>) -> Ray {
    let ndc = vec2(2.0 * pixel.x / f32(u.image.x) - 1.0, 1.0 - 2.0 * pixel.y / f32(u.image.y));
    if u.camera.w > 0.5 {
        let origin = u.cam_origin.xyz + u.cam_right.xyz * (ndc.x * u.camera.y * u.camera.z) + u.cam_up.xyz * (ndc.y * u.camera.z);
        return Ray(origin, normalize(u.cam_forward.xyz));
    }
    let direction = u.cam_forward.xyz + u.cam_right.xyz * (ndc.x * u.camera.y * u.camera.x) + u.cam_up.xyz * (ndc.y * u.camera.x);
    return Ray(u.cam_origin.xyz, normalize(direction));
}
fn viewport_background(ray: Ray) -> vec3<f32> {
    let t = smoothstep(-0.5, 0.7, ray.direction.y);
    return mix(vec3(0.029, 0.034, 0.043), vec3(0.018, 0.023, 0.032), t);
}
fn project_to_pixels(point: vec3<f32>) -> vec2<f32> {
    let delta = point - u.cam_origin.xyz;
    let half_height = select(u.camera.x * max(1e-5, dot(delta, u.cam_forward.xyz)), u.camera.z, u.camera.w > 0.5);
    return vec2(dot(delta, u.cam_right.xyz), dot(delta, u.cam_up.xyz))
        * vec2(f32(u.image.x) / u.camera.y, f32(u.image.y)) / (2.0 * half_height);
}
fn projected_edge_distance(position: vec3<f32>, start: vec3<f32>, end: vec3<f32>) -> f32 {
    var a = start; var b = end;
    if u.camera.w < 0.5 {
        let da = dot(a - u.cam_origin.xyz, u.cam_forward.xyz);
        let db = dot(b - u.cam_origin.xyz, u.cam_forward.xyz);
        let near_plane = 1e-4;
        if da < near_plane && db < near_plane { return FAR; }
        if da < near_plane { a = mix(a, b, (near_plane - da) / (db - da)); }
        else if db < near_plane { b = mix(b, a, (near_plane - db) / (da - db)); }
    }
    let pa = project_to_pixels(a); let pb = project_to_pixels(b);
    let point = project_to_pixels(position); let edge = pb - pa;
    let t = clamp(dot(point - pa, edge) / max(1e-12, dot(edge, edge)), 0.0, 1.0);
    return length(point - pa - t * edge);
}
fn edge_coverage(ray: Ray, hit: Hit, tri: Triangle, thickness: f32) -> f32 {
    let position = ray.origin + hit.distance * ray.direction;
    let mask = u32(tri.params.w + 0.5);
    var distance = FAR;
    if (mask & 1u) != 0u { distance = min(distance, projected_edge_distance(position, tri.v1.xyz, tri.v2.xyz)); }
    if (mask & 2u) != 0u { distance = min(distance, projected_edge_distance(position, tri.v2.xyz, tri.v0.xyz)); }
    if (mask & 4u) != 0u { distance = min(distance, projected_edge_distance(position, tri.v0.xyz, tri.v1.xyz)); }
    return 1.0 - smoothstep(thickness * 0.45, thickness * 1.45, distance);
}
fn is_selected(hit: Hit) -> bool {
    return hit.triangle != NO_HIT && u.settings.z > 0.0
        && abs(triangles[hit.triangle].params.z - u.settings.z) < 0.25;
}
fn selection_silhouette(pixel: vec2<f32>) -> f32 {
    let offsets = array<vec2<f32>, 4>(vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0));
    for (var i = 0u; i < 4u; i += 1u) {
        let neighbor = trace(camera_ray(pixel + offsets[i]), FAR, false);
        if !is_selected(neighbor) { return 1.0; }
    }
    return 0.0;
}
fn solid_shading(s: Surface, ray: Ray) -> vec3<f32> {
    let key = normalize(-u.cam_forward.xyz * 0.6 - u.cam_right.xyz * 0.45 + u.cam_up.xyz * 0.85);
    let fill = normalize(u.cam_right.xyz * 0.8 + u.cam_up.xyz * 0.2 - u.cam_forward.xyz * 0.25);
    let rim = normalize(u.cam_forward.xyz * 0.8 + u.cam_up.xyz * 0.6);
    let ambient = mix(0.15, 0.26, s.normal.y * 0.5 + 0.5);
    let shade = ambient + 0.65 * max(0.0, dot(s.normal, key)) + 0.17 * max(0.0, dot(s.normal, fill));
    let specular = pow(max(0.0, dot(s.normal, normalize(key - ray.direction))), 48.0) * 0.12;
    let rim_light = pow(max(0.0, dot(s.normal, rim)), 3.0) * 0.07;
    return vec3(0.39, 0.43, 0.49) * shade + vec3(specular + rim_light);
}
fn grid_line(coordinate: f32, spacing: f32, width: f32) -> f32 {
    let nearest = abs(coordinate - round(coordinate / spacing) * spacing);
    return 1.0 - smoothstep(width * 0.45, width * 1.35, nearest);
}
fn add_grid(initial_color: vec3<f32>, ray: Ray, hit: Hit, pixel: vec2<f32>) -> vec3<f32> {
    var color = initial_color;
    if u.settings.y < 0.5 || abs(ray.direction.y) < 1e-5 { return color; }
    let distance = -ray.origin.y / ray.direction.y;
    if distance <= 0.0 || distance > 10000.0 { return color; }
    let forward_depth = distance * max(0.01, dot(ray.direction, u.cam_forward.xyz));
    let pixel_world = 2.0 * select(forward_depth * u.camera.x, u.camera.z, u.camera.w > 0.5) / f32(u.image.y);
    let depth_tolerance = max(2e-5 * max(1.0, distance), pixel_world * 0.05 / max(0.1, abs(ray.direction.y)));
    if distance + depth_tolerance >= hit.distance { return color; }
    let p = ray.origin + distance * ray.direction;
    let rx = camera_ray(pixel + vec2(1.0, 0.0)); let ry = camera_ray(pixel + vec2(0.0, 1.0));
    if abs(rx.direction.y) < 1e-5 || abs(ry.direction.y) < 1e-5 { return color; }
    let px = rx.origin - rx.direction * (rx.origin.y / rx.direction.y);
    let py = ry.origin - ry.direction * (ry.origin.y / ry.direction.y);
    let wx = max(1e-6, length(vec2(px.x - p.x, py.x - p.x)));
    let wz = max(1e-6, length(vec2(px.z - p.z, py.z - p.z)));
    let footprint = max(wx, wz);
    let spacing = pow(10.0, floor(log2(max(0.001, footprint * 35.0)) / log2(10.0)));
    let minor = max(grid_line(p.x, spacing, wx), grid_line(p.z, spacing, wz));
    let major = max(grid_line(p.x, spacing * 10.0, wx), grid_line(p.z, spacing * 10.0, wz));
    let fade = (1.0 / (1.0 + distance * 0.025)) * smoothstep(0.015, 0.12, abs(ray.direction.y));
    let opacity = (0.11 * minor + 0.13 * major) * fade;
    color = mix(color, vec3(0.13, 0.15, 0.18), opacity);
    let x_axis = 1.0 - smoothstep(wz * 0.5, wz * 1.6, abs(p.z));
    let z_axis = 1.0 - smoothstep(wx * 0.5, wx * 1.6, abs(p.x));
    color = mix(color, vec3(0.23, 0.055, 0.06), x_axis * fade * 0.55);
    return mix(color, vec3(0.055, 0.12, 0.29), z_axis * fade * 0.55);
}
fn display_transform(linear: vec3<f32>, exposure: f32) -> vec3<f32> {
    let x = max(linear, vec3(0.0)) * exp2(clamp(exposure, -20.0, 20.0));
    let mapped = clamp((x * (2.51 * x + vec3(0.03))) / (x * (2.43 * x + vec3(0.59)) + vec3(0.14)), vec3(0.0), vec3(1.0));
    return select(1.055 * pow(mapped, vec3(1.0 / 2.4)) - vec3(0.055), 12.92 * mapped, mapped <= vec3(0.0031308));
}
fn wireframe_shading(ray: Ray, pixel: vec2<f32>, primary: Hit) -> vec3<f32> {
    var color = viewport_background(ray) * 1.10;
    color = add_grid(color, ray, Hit(FAR, vec2(0.0), NO_HIT), pixel);
    var march = ray; var hit = primary;
    for (var layer = 0u; layer < 4u; layer += 1u) {
        if layer > 0u { hit = trace(march, FAR, false); }
        if hit.triangle == NO_HIT { break; }
        let tri = triangles[hit.triangle];
        let edge = edge_coverage(march, hit, tri, 1.0);
        if edge > 0.0 {
            let selected = is_selected(hit);
            let line = select(vec3(0.82, 0.86, 0.92), vec3(0.055, 0.40, 0.95), selected);
            var alpha = select(edge * 0.35, edge, layer == 0u);
            if selected { alpha = select(edge * 0.45, edge * 0.92, layer == 0u); }
            color = mix(color, line, clamp(alpha, 0.0, 1.0));
        }
        let position = march.origin + march.direction * hit.distance;
        let step = max(1e-4, hit.distance * 1e-4);
        march.origin = position + ray.direction * step;
    }
    return color;
}
@compute @workgroup_size(8, 8)
fn render_main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let gid = global_id.xy;
    if gid.x >= u.image.x || gid.y >= u.image.y { return; }
    let index = gid.y * u.image.x + gid.x;
    var rng = hash_u32(index ^ hash_u32(u.image.z + 0x9e3779b9u));
    let progressive = u.image.w == 3u;
    var jitter = vec2(0.5);
    if progressive { jitter = random_pair(&rng); }
    let pixel = vec2<f32>(gid) + jitter;
    let ray = camera_ray(pixel); let primary = trace(ray, FAR, false);
    var color: vec3<f32>;
    if progressive { color = path_trace(ray, primary, &rng); }
    else if u.image.w == 0u { color = wireframe_shading(ray, pixel, primary); }
    else if primary.triangle != NO_HIT { color = solid_shading(surface_at(ray, primary), ray); }
    else { color = viewport_background(ray); }
    if u.image.w == 1u {
        color = add_grid(color, ray, primary, pixel);
        if is_selected(primary) {
            color = mix(color, vec3(0.055, 0.40, 0.95), selection_silhouette(pixel) * 0.92);
        }
    }
    if progressive {
        var total = vec4(color, 1.0);
        if u.image.z != 0u { total += accumulation[index]; }
        accumulation[index] = total;
        color = total.xyz / max(1.0, total.w);
    } else { accumulation[index] = vec4(color, 1.0); }
    textureStore(output, vec2<i32>(gid), vec4(display_transform(color, select(0.0, u.settings.x, progressive)), 1.0));
}

// Explicit filtering supports HDR textures on adapters without FLOAT32_FILTERABLE.
// Equirectangular maps repeat horizontally and clamp vertically at their poles.
fn env_texel_coord(pixel: vec2<i32>, size: vec2<u32>) -> vec2<i32> {
    let width = i32(size.x);
    return vec2(((pixel.x % width) + width) % width, clamp(pixel.y, 0, i32(size.y) - 1));
}
fn sample_env_level(tex: texture_2d<f32>, uv: vec2<f32>, level: i32) -> vec4<f32> {
    let size = textureDimensions(tex, level);
    let p = uv * vec2<f32>(size) - vec2(0.5);
    let base = vec2<i32>(floor(p)); let weight = fract(p);
    let a = textureLoad(tex, env_texel_coord(base, size), level);
    let b = textureLoad(tex, env_texel_coord(base + vec2(1, 0), size), level);
    let c = textureLoad(tex, env_texel_coord(base + vec2(0, 1), size), level);
    let d = textureLoad(tex, env_texel_coord(base + vec2(1, 1), size), level);
    return mix(mix(a, b, weight.x), mix(c, d, weight.x), weight.y);
}
fn sample_env(tex: texture_2d<f32>, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let last = i32(textureNumLevels(tex)) - 1;
    let level = clamp(lod, 0.0, f32(last)); let lower = i32(floor(level));
    return mix(sample_env_level(tex, uv, lower), sample_env_level(tex, uv, min(lower + 1, last)), fract(level));
}
fn sample_env_array(tex: texture_2d_array<f32>, uv: vec2<f32>, layer: i32) -> vec4<f32> {
    let size = textureDimensions(tex, 0);
    let p = uv * vec2<f32>(size) - vec2(0.5);
    let base = vec2<i32>(floor(p)); let weight = fract(p);
    let a = textureLoad(tex, env_texel_coord(base, size), layer, 0);
    let b = textureLoad(tex, env_texel_coord(base + vec2(1, 0), size), layer, 0);
    let c = textureLoad(tex, env_texel_coord(base + vec2(0, 1), size), layer, 0);
    let d = textureLoad(tex, env_texel_coord(base + vec2(1, 1), size), layer, 0);
    return mix(mix(a, b, weight.x), mix(c, d, weight.x), weight.y);
}
fn sample_lut(tex: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = textureDimensions(tex, 0);
    let p = uv * vec2<f32>(size) - vec2(0.5);
    let base = vec2<i32>(floor(p)); let weight = fract(p); let last = vec2<i32>(size) - vec2(1);
    let a = textureLoad(tex, clamp(base, vec2(0), last), 0);
    let b = textureLoad(tex, clamp(base + vec2(1, 0), vec2(0), last), 0);
    let c = textureLoad(tex, clamp(base + vec2(0, 1), vec2(0), last), 0);
    let d = textureLoad(tex, clamp(base + vec2(1, 1), vec2(0), last), 0);
    return mix(mix(a, b, weight.x), mix(c, d, weight.x), weight.y);
}
