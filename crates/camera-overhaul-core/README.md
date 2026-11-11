# camera-overhaul-core (Rust port)

Engine-independent Rust port of the camera logic of
[CameraOverhaul](https://github.com/Mirsario/Minecraft-CameraOverhaul) by Mirsario & Contributors.
GPL-3.0 (same as the original). Keep the LICENSE and the credit if you reuse this.

Ported: turning roll, strafing roll, forward/vertical velocity pitch, idle camera sway,
camera (mouse) smoothing, per-context settings (walking, sprinting, swimming, elytra,
mount, vehicle), screen shakes (explosion, lightning, hand swing).

Not ported: the config screen and the mixins (they are Java/Minecraft specific).

## Using it

```rust
let mut co = camera_overhaul_core::CameraOverhaul::new(Default::default());

// every rendered frame (skip while paused):
let off = co.update(&ctx, delta_seconds);
camera.pitch += off.pitch as f32;
camera.yaw   += off.yaw   as f32;
camera.roll  += off.roll  as f32;   // roll is applied after the camera matrix

// events:
co.on_explosion([x, y, z]);
co.on_lightning([x, y, z]);
co.on_hand_swing();
```

Needs from the host: player velocity (blocks/tick), yaw/pitch, sprint/swim/elytra/mount flags,
and a place in the camera code to apply pitch, yaw and roll.

Note: the simplex noise is a standard implementation, so the sway looks the same in style
but is not bit-identical to JOML's.
