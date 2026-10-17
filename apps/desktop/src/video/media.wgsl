// PQ/HLG media only. The ordinary UI follows egui's separate SDR-white path.
struct Params { levels: vec4<u32>, color: vec4<u32>, geometry: vec4<u32> };
@group(0) @binding(0) var y_plane: texture_2d<u32>;
@group(0) @binding(1) var uv_plane: texture_2d<u32>;
@group(0) @binding(2) var<uniform> p: Params;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let positions = array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(-1.0,3.0),vec2(3.0,-1.0));
    var result: Vertex;
    result.position=vec4(positions[index],0.0,1.0);
    result.uv=vec2((positions[index].x+1.0)*0.5,(1.0-positions[index].y)*0.5);
    return result;
}
fn linear_srgb(rgb: vec3<f32>) -> vec3<f32> {
    return select(pow((rgb+vec3(0.055))/1.055,vec3(2.4)),rgb/12.92,rgb<=vec3(0.04045));
}
fn gamma_srgb(rgb: vec3<f32>) -> vec3<f32> {
    return select(1.055*pow(max(rgb,vec3(0.0)),vec3(1.0/2.4))-vec3(0.055),rgb*12.92,rgb<=vec3(0.0031308));
}
fn pq_nits(rgb: vec3<f32>) -> vec3<f32> {
    let x=pow(clamp(rgb,vec3(0.0),vec3(1.0)),vec3(32.0/2523.0));
    return 10000.0*pow(max(x-vec3(3424.0/4096.0),vec3(0.0))/max(vec3(2413.0/128.0)-(2392.0/128.0)*x,vec3(0.000001)),vec3(16384.0/2610.0));
}
fn hlg_nits(rgb: vec3<f32>,peak:f32)->vec3<f32> {
    let e=clamp(rgb,vec3(0.0),vec3(1.0));
    let scene=select((exp((e-vec3(0.55991073))/0.17883277)+vec3(0.28466892))/12.0,e*e/3.0,e<=vec3(0.5));
    let gamma=1.2+0.42*log2(peak/1000.0)/log2(10.0);
    let luminance=max(dot(scene,vec3(0.2627,0.6780,0.0593)),0.0000001);
    return scene*pow(luminance,gamma-1.0)*peak;
}
// Integer P010 textures are sampled explicitly so resize/downscale does not
// alias chroma or depend on optional integer-texture filtering capabilities.
fn bilinear_y(uv: vec2<f32>) -> f32 {
    let size = textureDimensions(y_plane);
    let pos = clamp(uv * vec2<f32>(size) - vec2(0.5), vec2(0.0), vec2<f32>(size)-vec2(1.0));
    let a = vec2<i32>(pos); let b = min(a+vec2(1),vec2<i32>(size)-vec2(1));
    let f = fract(pos);
    return mix(mix(f32(textureLoad(y_plane,a,0).r>>6u),f32(textureLoad(y_plane,vec2(b.x,a.y),0).r>>6u),f.x),
               mix(f32(textureLoad(y_plane,vec2(a.x,b.y),0).r>>6u),f32(textureLoad(y_plane,b,0).r>>6u),f.x),f.y);
}
fn bilinear_uv(uv: vec2<f32>) -> vec2<f32> {
    let size = textureDimensions(uv_plane);
    // Odd visible crops retain a partial final chroma sample. Map from luma
    // geometry, rather than stretching the rounded-up chroma texture.
    // FFmpeg's unspecified 4:2:0 location defaults to MPEG left siting.
    // Respect explicit center, top and bottom positions from the sequence header.
    let location = p.color.w >> 8u;
    var offset = vec2(0.25, 0.5);
    if location==2u || location==4u || location==6u { offset.x=0.5; }
    if location==3u || location==4u { offset.y=0.25; }
    if location==5u || location==6u { offset.y=0.75; }
    let pos = clamp(uv * vec2<f32>(textureDimensions(y_plane)) * 0.5 - offset, vec2(0.0), vec2<f32>(size)-vec2(1.0));
    let a = vec2<i32>(pos); let b = min(a+vec2(1),vec2<i32>(size)-vec2(1));
    let f = fract(pos);
    return mix(mix(vec2<f32>(textureLoad(uv_plane,a,0).rg>>vec2(6u)),vec2<f32>(textureLoad(uv_plane,vec2(b.x,a.y),0).rg>>vec2(6u)),f.x),
               mix(vec2<f32>(textureLoad(uv_plane,vec2(a.x,b.y),0).rg>>vec2(6u)),vec2<f32>(textureLoad(uv_plane,b,0).rg>>vec2(6u)),f.x),f.y);
}
fn bilinear_rgba(uv: vec2<f32>) -> vec4<f32> {
    let size = textureDimensions(y_plane);
    let pos = clamp(uv * vec2<f32>(size) - vec2(0.5), vec2(0.0), vec2<f32>(size)-vec2(1.0));
    let a = vec2<i32>(pos); let b = min(a+vec2(1),vec2<i32>(size)-vec2(1));
    let f = fract(pos);
    return mix(mix(vec4<f32>(textureLoad(y_plane,a,0)),vec4<f32>(textureLoad(y_plane,vec2(b.x,a.y),0)),f.x),
               mix(vec4<f32>(textureLoad(y_plane,vec2(a.x,b.y),0)),vec4<f32>(textureLoad(y_plane,b,0)),f.x),f.y)/255.0;
}
fn sample_yuv(uv:vec2<f32>)->vec3<f32>{
    let y = bilinear_y(uv); let c = bilinear_uv(uv);
    var yy=(y-64.0)/876.0;
    var cc=(c-vec2(512.0))/896.0;
    if (p.color.w&1u)!=0u{yy=y/1023.0;cc=(c-vec2(512.0))/1023.0;}
    var kr=0.2627;var kb=0.0593;
    if p.color.z==1u{kr=0.2126;kb=0.0722;}
    if p.color.z==5u || p.color.z==6u{kr=0.299;kb=0.114;}
    let kg=1.0-kr-kb;
    return vec3(yy+2.0*(1.0-kr)*cc.y,yy-2.0*kb*(1.0-kb)/kg*cc.x-2.0*kr*(1.0-kr)/kg*cc.y,yy+2.0*(1.0-kb)*cc.x);
}
@fragment fn fragment(in:Vertex)->@location(0) vec4<f32>{
    var uv=in.uv;
    if p.geometry.z==90u{uv=vec2(uv.y,1.0-uv.x);}
    if p.geometry.z==180u{uv=vec2(1.0)-uv;}
    if p.geometry.z==270u{uv=vec2(1.0-uv.y,uv.x);}
    if p.geometry.w==0u {
        let rgba=bilinear_rgba(uv);
        return vec4(select(rgba.rgb,linear_srgb(rgba.rgb)*bitcast<f32>(p.geometry.x),p.levels.w!=0u),rgba.a);
    }
    let white=bitcast<f32>(p.levels.x);
    let peak=bitcast<f32>(p.levels.y);
    let headroom=bitcast<f32>(p.levels.z);
    let encoded=sample_yuv(uv);
    var rgb=pq_nits(encoded)/white;
    if p.color.x==18u{rgb=hlg_nits(encoded,peak)/white;}
    if p.color.y==9u {
        rgb=vec3(dot(rgb,vec3(1.660491,-0.587641,-0.072850)),dot(rgb,vec3(-0.124550,1.132900,-0.008349)),dot(rgb,vec3(-0.018151,-0.100579,1.118730)));
    } else if p.color.y==12u {
        rgb=vec3(dot(rgb,vec3(1.224940,-0.224940,0.0)),dot(rgb,vec3(-0.042057,1.042057,0.0)),dot(rgb,vec3(-0.019638,-0.078636,1.098274)));
    }
    let luminance=max(dot(rgb,vec3(0.2126,0.7152,0.0722)),0.0);
    let display_peak=max(headroom,1.0);
    let knee=display_peak*0.8;
    if peak/white>display_peak && luminance>knee {
        let mapped=knee+(display_peak-knee)*(1.0-exp(-(luminance-knee)/(display_peak-knee)));
        rgb*=mapped/max(luminance,0.000001);
    }
    if p.levels.w!=1u {
        let l=clamp(dot(rgb,vec3(0.2126,0.7152,0.0722)),0.0,1.0);
        let lo=min(rgb.r,min(rgb.g,rgb.b));let hi=max(rgb.r,max(rgb.g,rgb.b));
        var saturation=1.0;
        if lo<0.0{saturation=min(saturation,l/max(l-lo,0.000001));}
        if hi>1.0{saturation=min(saturation,(1.0-l)/max(hi-l,0.000001));}
        rgb=vec3(l)+(rgb-vec3(l))*saturation;
        rgb=clamp(rgb,vec3(0.0),vec3(1.0));
        return vec4(select(gamma_srgb(rgb),rgb,p.levels.w==2u),1.0);
    }
    return vec4(rgb,1.0);
}
