//! Direct3D's immediate mode.
//!
//! What the roster's titles actually draw through: they take the device
//! underneath retained mode and hand it primitives themselves. Everything it
//! draws lands in the guest's own surface (ADR-0002), so what the guest reads
//! back is what was drawn.

use super::{ComState, Ctx, State};
use crate::com::{Interface, Method, ADD_REF, QUERY_INTERFACE, RELEASE, S_OK};
use crate::cpu::Stop;
use crate::guest::GuestMem;

/// `IDirect3DDevice2`, in slot order.
pub static DEVICE2: Interface = Interface {
    name: "IDirect3DDevice2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "GetCaps",
            args: 2,
            run: caps,
        },
        Method {
            name: "SwapTextureHandles",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetStats",
            args: 1,
            run: ok,
        },
        Method {
            name: "AddViewport",
            args: 1,
            run: add_viewport,
        },
        Method {
            name: "DeleteViewport",
            args: 1,
            run: ok,
        },
        Method {
            name: "NextViewport",
            args: 3,
            run: ok,
        },
        Method {
            name: "EnumTextureFormats",
            args: 2,
            run: ok,
        },
        Method {
            name: "BeginScene",
            args: 0,
            run: ok,
        },
        Method {
            name: "EndScene",
            args: 0,
            run: end_scene,
        },
        Method {
            name: "GetDirect3D",
            args: 1,
            run: get_direct3d,
        },
        Method {
            name: "SetCurrentViewport",
            args: 1,
            run: set_viewport,
        },
        Method {
            name: "GetCurrentViewport",
            args: 1,
            run: get_viewport,
        },
        Method {
            name: "SetRenderTarget",
            args: 2,
            run: set_render_target,
        },
        Method {
            name: "GetRenderTarget",
            args: 1,
            run: get_render_target,
        },
        Method {
            name: "Begin",
            args: 3,
            run: ok,
        },
        Method {
            name: "BeginIndexed",
            args: 5,
            run: ok,
        },
        Method {
            name: "Vertex",
            args: 1,
            run: ok,
        },
        Method {
            name: "Index",
            args: 1,
            run: ok,
        },
        Method {
            name: "End",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetRenderState",
            args: 2,
            run: get_state,
        },
        Method {
            name: "SetRenderState",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetLightState",
            args: 2,
            run: get_state,
        },
        Method {
            name: "SetLightState",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetTransform",
            args: 2,
            run: ok,
        },
        Method {
            name: "GetTransform",
            args: 2,
            run: ok,
        },
        Method {
            name: "MultiplyTransform",
            args: 2,
            run: ok,
        },
        Method {
            name: "DrawPrimitive",
            args: 5,
            run: draw,
        },
        Method {
            name: "DrawIndexedPrimitive",
            args: 7,
            run: draw_indexed,
        },
        Method {
            name: "SetClipStatus",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetClipStatus",
            args: 1,
            run: ok,
        },
    ],
};

/// `IDirect3D2`, in slot order.
pub static D3D2: Interface = Interface {
    name: "IDirect3D2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "EnumDevices",
            args: 2,
            run: ok,
        },
        Method {
            name: "CreateLight",
            args: 2,
            run: make_object,
        },
        Method {
            name: "CreateMaterial",
            args: 2,
            run: make_object,
        },
        Method {
            name: "CreateViewport",
            args: 2,
            run: make_viewport,
        },
        Method {
            name: "FindDevice",
            args: 2,
            run: ok,
        },
        Method {
            name: "CreateDevice",
            args: 3,
            run: ok,
        },
    ],
};

/// `IDirect3DViewport2`, in slot order.
pub static VIEWPORT2: Interface = Interface {
    name: "IDirect3DViewport2",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Initialize",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetViewport",
            args: 1,
            run: get_viewport_data,
        },
        Method {
            name: "SetViewport",
            args: 1,
            run: set_viewport_data,
        },
        Method {
            name: "TransformVertices",
            args: 5,
            run: ok,
        },
        Method {
            name: "LightElements",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetBackground",
            args: 1,
            run: set_background,
        },
        Method {
            name: "GetBackground",
            args: 2,
            run: ok,
        },
        Method {
            name: "SetBackgroundDepth",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetBackgroundDepth",
            args: 2,
            run: ok,
        },
        Method {
            name: "Clear",
            args: 3,
            run: clear,
        },
        Method {
            name: "AddLight",
            args: 1,
            run: ok,
        },
        Method {
            name: "DeleteLight",
            args: 1,
            run: ok,
        },
        Method {
            name: "NextLight",
            args: 3,
            run: ok,
        },
        Method {
            name: "GetViewport2",
            args: 1,
            run: get_viewport_data,
        },
        Method {
            name: "SetViewport2",
            args: 1,
            run: set_viewport_data,
        },
    ],
};

/// Everything else immediate mode hands out and this machine only has to keep
/// distinct: lights, materials and textures.
pub static THING: Interface = Interface {
    name: "IDirect3DThing",
    methods: &[
        QUERY_INTERFACE,
        ADD_REF,
        RELEASE,
        Method {
            name: "Initialize",
            args: 1,
            run: ok,
        },
        Method {
            name: "SetLight",
            args: 1,
            run: ok,
        },
        Method {
            name: "GetLight",
            args: 1,
            run: ok,
        },
    ],
};

pub static INTERFACES: &[&Interface] = &[&DEVICE2, &D3D2, &VIEWPORT2, &THING];

fn ok(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}

fn get_state(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    if args[1] != 0 {
        ctx.mem.write_u32(args[1], 0).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn caps(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    // What the declared machine's 3D hardware can do. Everything this
    // rasteriser does is written here, so what is claimed is what is true.
    for at in [args[0], args[1]] {
        if at == 0 {
            continue;
        }
        let size = ctx.mem.read_u32(at).map_err(Stop::Fault)?.clamp(4, 0x200);
        for offset in (4..size).step_by(4) {
            ctx.mem.write_u32(at + offset, 0).map_err(Stop::Fault)?;
        }
    }
    Ok(S_OK)
}

fn make_object(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &THING, ComState::None).map_err(Stop::Fault)?;
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn make_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(
        ctx,
        &VIEWPORT2,
        ComState::Viewport {
            device: 0,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            background: 0,
        },
    )
    .map_err(Stop::Fault)?;
    if args[0] != 0 {
        ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn get_direct3d(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let object = State::make_com(ctx, &D3D2, ComState::None).map_err(Stop::Fault)?;
    ctx.mem.write_u32(args[0], object).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn add_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // A viewport belongs to the device that took it, and draws into that
    // device's surface.
    if let Some(object) = ctx.state.com_mut(args[0]) {
        if let ComState::Viewport { device, .. } = &mut object.state {
            *device = this;
        }
    }
    Ok(S_OK)
}

fn set_viewport(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Device3D { .. } = &object.state {
            // The device remembers which viewport is current by handle.
            ctx.state.current_viewport = args[0];
        }
    }
    Ok(S_OK)
}

fn get_viewport(ctx: &mut Ctx<'_>, _this: u32, args: &[u32]) -> Result<u32, Stop> {
    let current = ctx.state.current_viewport;
    ctx.mem.write_u32(args[0], current).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn set_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // The surface the device draws into, changed after it was made.
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Device3D { surface } = &mut object.state {
            *surface = args[0];
        }
    }
    ctx.log
        .line(&format!("d3d render target {:#010x}", args[0]));
    Ok(S_OK)
}

fn get_render_target(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let surface = match ctx.state.com(this).map(|o| o.state.clone()) {
        Some(ComState::Device3D { surface }) => surface,
        _ => 0,
    };
    ctx.mem.write_u32(args[0], surface).map_err(Stop::Fault)?;
    Ok(S_OK)
}

fn get_viewport_data(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    let Some(ComState::Viewport {
        x,
        y,
        width,
        height,
        ..
    }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    let at = args[0];
    for (offset, value) in [(4, x), (8, y), (12, width), (16, height)] {
        ctx.mem.write_u32(at + offset, value).map_err(Stop::Fault)?;
    }
    Ok(S_OK)
}

fn set_viewport_data(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // D3DVIEWPORT: size, then x, y, width and height.
    let at = args[0];
    let x = ctx.mem.read_u32(at + 4).map_err(Stop::Fault)?;
    let y = ctx.mem.read_u32(at + 8).map_err(Stop::Fault)?;
    let width = ctx.mem.read_u32(at + 12).map_err(Stop::Fault)?;
    let height = ctx.mem.read_u32(at + 16).map_err(Stop::Fault)?;
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Viewport {
            x: vx,
            y: vy,
            width: vw,
            height: vh,
            ..
        } = &mut object.state
        {
            *vx = x;
            *vy = y;
            *vw = width;
            *vh = height;
        }
    }
    ctx.log
        .line(&format!("d3d viewport {width}x{height} at {x},{y}"));
    Ok(S_OK)
}

fn set_background(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    if let Some(object) = ctx.state.com_mut(this) {
        if let ComState::Viewport { background, .. } = &mut object.state {
            *background = args[0];
        }
    }
    Ok(S_OK)
}

fn clear(ctx: &mut Ctx<'_>, this: u32, args: &[u32]) -> Result<u32, Stop> {
    // Clear(count, rects, flags). This is the first thing a frame is made of,
    // and the first thing this machine can put on a screen.
    let Some(ComState::Viewport {
        device,
        x,
        y,
        width,
        height,
        background,
    }) = ctx.state.com(this).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    let Some(ComState::Device3D { surface }) = ctx.state.com(device).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };
    let Some(ComState::Surface {
        pixels,
        pitch,
        depth,
        width: sw,
        height: sh,
        ..
    }) = ctx.state.com(surface).map(|o| o.state.clone())
    else {
        return Ok(S_OK);
    };

    // The background is an ARGB colour; the surface may be narrower than that.
    let colour = match depth {
        16 => {
            let [b, g, r, _] = background.to_le_bytes();
            u32::from((u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3))
        }
        _ => background,
    };
    let bytes = depth.div_ceil(8);
    let x1 = (x + width).min(sw).max(x);
    let y1 = (y + height).min(sh).max(y);
    let row: Vec<u8> = (x..x1)
        .flat_map(|_| colour.to_le_bytes().into_iter().take(bytes as usize))
        .collect();
    for row_y in y..y1 {
        ctx.mem
            .write(pixels + row_y * pitch + x * bytes, &row)
            .map_err(Stop::Fault)?;
    }
    let _ = args;
    ctx.state.cleared += 1;
    Ok(S_OK)
}

fn end_scene(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // The scene is finished on the device's surface; the title presents it by
    // flipping, which is where the machine counts a frame.
    Ok(S_OK)
}

fn draw(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    // Triangles are the rasteriser's, and it arrives with #12. What is on the
    // surface until then is what Clear left there.
    Ok(S_OK)
}

fn draw_indexed(_ctx: &mut Ctx<'_>, _this: u32, _args: &[u32]) -> Result<u32, Stop> {
    Ok(S_OK)
}
