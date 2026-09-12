// GPUI 0.2.2's surface compositor requires full-range bi-planar 4:2:0.
// Match its BT.601 inverse exactly. The RGBA film is already sRGB encoded.
// One thread owns a 2x2 luma block and its averaged chroma sample, avoiding races.
kernel void rgba_to_nv12(texture2d<float, access::read> rgba [[texture(0)]],
                         texture2d<float, access::write> y_plane [[texture(1)]],
                         texture2d<float, access::write> uv_plane [[texture(2)]],
                         uint2 gid [[thread_position_in_grid]]) {
    if (gid.x >= uv_plane.get_width() || gid.y >= uv_plane.get_height()) return;
    float3 mean = 0.0f;
    for (uint y = 0; y < 2; ++y) {
        for (uint x = 0; x < 2; ++x) {
            uint2 p = gid * 2 + uint2(x, y);
            float3 rgb = rgba.read(p).rgb;
            y_plane.write(float4(dot(rgb, float3(0.299f, 0.587f, 0.114f)), 0, 0, 1), p);
            mean += rgb * 0.25f;
        }
    }
    float cb = dot(mean, float3(-0.168736f, -0.331264f, 0.5f)) + 0.5f;
    float cr = dot(mean, float3(0.5f, -0.418688f, -0.081312f)) + 0.5f;
    uv_plane.write(float4(cb, cr, 0, 1), gid);
}
