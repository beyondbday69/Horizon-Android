# barrel-roll-core (Rust port)

Engine-independent Rust port of the client-side flight rotation of
[Do a Barrel Roll](https://github.com/enjarai/do-a-barrel-roll) by enjarai,
which is based on Cool Elytra Roll by Jorbon.
GPL-3.0 (same as the original). Keep the LICENSE and the credit if you reuse this.

Ported: the free orientation while gliding (look input rolls and pitches, strafe
input yaws), banking, smoothing, automatic righting, and the way the player's real
yaw and pitch are derived from the forward vector.

Not ported: thrusting, the momentum-based mouse, the activation-behaviour modes,
the config screen, the horizon widget and every server-side or sync feature.

## Using it

```rust
let mut roll = barrel_roll_core::BarrelRoll::new(Default::default());
roll.set_orientation(player_yaw, player_pitch);      // when gliding starts

// every frame while gliding:
let o = roll.update(barrel_roll_core::Input {
    pitch: dy,      // look input along -Y (mouse or touch drag)
    roll: dx,       // look input along +X
    yaw: strafe,    // strafe input, right positive
}, delta_seconds);
player_yaw = o.yaw;    // the engine flies where you look
player_pitch = o.pitch;
camera_roll = -o.roll; // the view banks (CameraMixin negates the entity roll)

// after flight ends, blend the camera upright without a snap:
while roll.settle(delta_seconds).abs() > 0.0 { /* render the fading roll */ }
```

Needs from the host: the look input and the strafe input each frame, the render
delta, a place to set the player's yaw/pitch, and the existing camera roll.

Note: the smoothing filter is a standard first-order low-pass, so the feel is the
same in style but not bit-identical to Minecraft's `Smoother`.
