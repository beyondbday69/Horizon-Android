# sound-physics-core (Rust port)

Engine-independent Rust port of the occlusion and reverberation model of
[Sound Physics Remastered](https://github.com/henkelmax/sound-physics-remastered)
by Sonic Ether and henkelmax, which is based on Sound Physics by Sonic Ether.
GPL-3.0 (same as the original). Keep the LICENSE and the credit if you reuse this.

Ported: the per-sound environment evaluation — how much the blocks between the
source and the listener absorb (occlusion, nine probe paths), the direct gain and
low-pass cutoff, the four reverb sends with their cutoffs, the room's shared
airspace and the position the sound is heard from, the four reverb presets, the
sound rate limit, the config with the original's defaults, and the block material
tables ([`blocks`]: occlusion and reflectivity by block name, as the host has no
sound types).

Not ported: the OpenAL EFX calls (the host turns [`ReverbParams`] into its own
reverb), the per-block config files (the host supplies [`SoundWorld`]), the debug
ray rendering, and the voice-chat integration.

## Using it

```rust
use sound_physics_core::{Config, default_environment, evaluate, SoundWorld};

// The host implements SoundWorld over its own world and ray-caster.
let env = evaluate(world, sound_pos, listener_pos, listener_underwater,
                   &config, /* direct gain scale */ 1.0,
                   /* the source's max distance */ 16.0,
                   /* rate limited this tick */ false);

channel.gain *= env.direct_gain;
channel.low_pass = env.direct_cutoff;      // AL_LOWPASS_GAINHF
for (bus, send) in env.sends.iter().enumerate() {
    reverb.feed(bus, send.gain, send.cutoff);
}
```

Note: the ray-casts and block lookups come from the host, so the maths here is
testable against a mock world; the four reverb presets' values are the original's.
