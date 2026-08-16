#include <metal_stdlib>

using namespace metal;

struct AlpineQuad {
    float4 bounds;
    float4 color;
    float4 atlas_uv;
};

struct AlpineVertexOutput {
    float4 position [[position]];
    float4 color;
    float2 atlas_uv;
};

vertex AlpineVertexOutput alpine_quad_vertex(
    uint vertex_id [[vertex_id]],
    uint instance_id [[instance_id]],
    constant float2 &viewport [[buffer(0)]],
    constant AlpineQuad *quads [[buffer(1)]])
{
    constexpr float2 corners[6] = {
        float2(0.0, 0.0),
        float2(1.0, 0.0),
        float2(0.0, 1.0),
        float2(0.0, 1.0),
        float2(1.0, 0.0),
        float2(1.0, 1.0),
    };

    const AlpineQuad quad = quads[instance_id];
    const float2 extent = quad.bounds.zw - quad.bounds.xy;
    const float2 pixel = quad.bounds.xy + corners[vertex_id] * extent;
    const float2 normalized = pixel / viewport;

    AlpineVertexOutput output;
    output.position = float4(
        normalized.x * 2.0 - 1.0,
        1.0 - normalized.y * 2.0,
        0.0,
        1.0);
    output.color = quad.color;
    output.atlas_uv = mix(quad.atlas_uv.xy, quad.atlas_uv.zw, corners[vertex_id]);
    return output;
}

fragment float4 alpine_quad_fragment(
    AlpineVertexOutput input [[stage_in]],
    texture2d<float> atlas [[texture(0)]])
{
    constexpr sampler nearest(coord::normalized, address::clamp_to_edge, filter::nearest);
    const float coverage = input.atlas_uv.x < 0.0 ? 1.0 : atlas.sample(nearest, input.atlas_uv).r;
    const float alpha = input.color.a * coverage;
    return float4(input.color.rgb * alpha, alpha);
}
