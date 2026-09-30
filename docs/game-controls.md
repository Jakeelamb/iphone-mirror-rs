# Game controls

Game mode translates desktop input into touchscreen contacts. It does not use
the game's internal APIs or inspect its state. HUD positions and the game's
own hold/toggle settings determine what each press does.

For proposed additions and a Mobile settings audit, see the dated
[PC controls comparison](pc-controls-comparison.md). It describes research and
future work, not additional supported bindings.

```sh
iphone-mirror-rs --connection wifi --game-profile ~/.config/iphone-mirror-rs/rainbow-six.profile
```

The normal mirror controls stay active until you press **Escape** (or F8). A new profile
needs calibration first. Use the game's training area or HUD editor where its
controls are visible; menu buttons are not the gameplay HUD.

## Calibrate

Press **F9** and click each target requested by the status strip above Home:
joystick center, an empty area for looking, lean left, lean right, reload,
interact, melee, grenade, primary weapon, secondary weapon, crouch, fire, aim.
These calibration clicks stay in the viewer; they do not tap the phone.
Escape cancels. The final click saves the profile atomically. F9 repeats the
calibration; different layouts can use different profile files.

To map an input without repeating setup, press **F10**, press the desired key,
mouse button, or wheel direction, then **left-click its on-screen control**.
Mouse inputs need two steps: the first click selects the input; the next left
click sets the target. Calibration clicks stay local and never tap the phone.
Any physical key reported by the viewer, middle/side mouse buttons, additional
numbered mouse buttons, and vertical/horizontal wheel directions are supported.
Escape and F8/F9/F10 remain reserved for release and setup.

Previously unbound inputs create an independent HUD target, saved as a
`custom0` through `custom63` action. Repeating F10 for that input moves its
existing target. There are 64 custom targets and at most 512 explicit bindings.
Existing bindings retain their action: for example, M calibrates mount, WASD
calibrates the joystick center, and Shift calibrates sprint. To change an
existing key's action or add aliases, use the profile overrides below.

The default optional actions are **Space** vault/climb, **X** rappel, **B** second
gadget, and **M** mount. They remain inactive until calibrated; the viewer sends
no guessed touch. F10 saves and applies immediately. Escape cancels without
saving. Contextual actions must be visible at an eligible ledge/wall or exposed
in the game's HUD editor.

For sprint, use **F10 → Shift → click the forward sprint endpoint above the
joystick**. Its vertical distance replaces the default sprint multiplier;
the joystick center remains unchanged. Choose the point the movement finger
must reach, rather than a separate action button. Test in the training area
with ADS/crouch off. This calibration changes the drag endpoint, not the game's
rules, and does not intentionally release on the sprint-lock icon.

Coordinates are relative to the displayed phone image, excluding margins and
the Home strip. Resize the window freely. Recalibrate after moving game HUD
buttons. Rotation or a change of video dimensions exits game mode; re-enter
only when the corresponding HUD is displayed.

Some controls are contextual or hidden by the game's settings. Expose them in
the HUD editor before calibration, or edit their coordinates in the profile.
Mapping two keys to the same weapon-toggle button will toggle for both keys;
direct primary/secondary selection requires two distinct on-screen targets.

For Rainbow Six Mobile's Pro layout, enable the **Melee** button in the HUD
editor and set both **Double Tap Effect** settings to **No Double Tap Action**.
Otherwise repeated joystick presses or look recentering can trigger ping/melee
shortcuts. The viewer does not change these game settings automatically.

Enable **Lock Joystick in Place** so its origin matches your calibration.
For manual firing, set **Gameplay → Input → ADS Auto-Shoot Behavior** and
**Hip Fire Auto-Shoot Behavior** to **Off**. Hip-fire auto-shoot can also enter
ADS when a target is out of range; that behavior comes from the game.
For leaning without ADS, enable the game's **Hip Lean** option in Gameplay
Settings, described in [Ubisoft's Hip Lean release notes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2388-patch-notes---1-5-toxic-fog/?p=web).
The viewer sends the calibrated lean touch; it does not separately press ADS
when Q/E are used. Recalibrate if changing this option changes HUD positions.

## Play

Focus the mirror and press **Escape**. The cursor locks and hides. Press Escape
again to release every contact and restore the pointer; press it once more to
resume with fresh contact state. F8 remains an alternate toggle. During
calibration, Escape only cancels setup. Focus loss also exits. F1 releases game
controls before sending Home. The viewer refuses capture if pointer locking is
unavailable, rather than letting aiming stop at the desktop edge.
Exiting discards queued game gestures and releases the contacts already sent
to the phone, so pending look recenters do not replay after Escape.

| Input | Touch target |
| --- | --- |
| W / A / S / D | Movement joystick; diagonals have the same radius |
| Hold Shift + W (optionally A / D) | Extend forward joystick displacement for sprint |
| Q / E | Lean left / right |
| R | Reload |
| F | Interact |
| V | Melee |
| G | Grenade |
| B | Second throwable / gadget (requires its own calibration) |
| Space | Vault / climb (contextual; requires calibration) |
| X | Rappel (contextual; requires calibration) |
| M | Mount (requires its own calibration) |
| 1 / 2 | Primary / secondary weapon |
| C | Crouch |
| Left / right mouse button | Fire / aim down sights |
| Mouse motion | Drag in the calibrated looking area |

A bound button stays touched until its key/button is released. Choose hold or
toggle behavior in the game's settings. Multiple inputs bound to the same
action share one contact; it lifts only when every held alias is released.
Wheel bindings send a 30 ms tap; repeated ticks extend that tap, rather than
queueing future taps. Wheel directions cannot bind movement or sprint. Unbound
inputs are suppressed while captured, so keys cannot type into a text field.
Exit game mode to use menus or normal keyboard input.

## Rebind keys and mouse buttons

Profiles accept `bind.INPUT=ACTION` overrides alongside target coordinates.
Input names use physical keyboard codes (`key.KeyT`, `key.ControlLeft`,
`key.ArrowUp`, `key.Numpad7`, `key.F12`), mouse names (`mouse.Left`, `Right`,
`Middle`, `Back`, `Forward`, `Other8`, each prefixed with `mouse.`), or
`wheel.Up`, `Down`, `Left`, `Right`, each prefixed with `wheel.`. F10's title
shows the selected input name for a new custom binding. Keys intercepted by
the desktop or not reported by the input backend cannot reach the viewer.

Actions are `up`, `down`, `left`, `right`, `sprint`, the named button fields
(`fire`, `aim`, `reload`, `mount`, etc.), or a calibrated `custom0`..`custom63`.
Use `none` to disable an input. For example:

```ini
# Add arrow movement and a mouse-side reload alias.
bind.key.ArrowUp=up
bind.mouse.Back=reload
# Replace the old R reload key and add a middle-button ability target.
bind.key.KeyR=none
bind.mouse.Middle=custom0
custom0=0.8,0.3
# Use wheel directions for calibrated weapon slots.
bind.wheel.Up=primary
bind.wheel.Down=secondary
```

The example coordinate is illustrative; use F10 or your actual HUD coordinate.
Removing an override restores that input's default action. Editing the file
requires restarting the viewer; F10 calibration does not. Duplicate inputs,
unknown actions/keys and reserved-key overrides are rejected. F1/F2 retain
Home/Spotlight unless explicitly bound in game mode. Distinct actions mapped
to the same coordinate are separate touches; use the same action name for
aliases. The five-contact limit still applies, including three button contacts.

Releasing WASD immediately centers the joystick, then lifts its touch after
150 ms with no movement keys held. A direction change within that grace period
reuses the origin; a longer pause starts a fresh gesture. This prevents idle
movement touches from persisting indefinitely across gameplay screen changes.
The viewer does not detect death/respawn; if controls still become stuck, use
Escape twice to reset all contacts and resume. Escape, focus loss and leaving game mode
always lift contacts immediately.
Either Shift key works; sprint stays requested until both are released. Shift
alone does not start movement. Releasing Shift restores normal displacement
without lifting the movement contact or disturbing look/lean/fire contacts.
The game decides whether the larger displacement activates sprint, including
its restrictions while aiming, crouching or moving diagonally.

## Tuning and limits

Profiles are small `name=value` text files with `version=1`. Targets use
`name=x,y`, with both coordinates between 0 and 1. Calibration fills them in.
`sensitivity=0.002` is the fraction of the phone's displayed short edge per
raw relative mouse unit (device/backend dependent, not screen pixels). Lower it
for slower aiming. `joystick_radius=0.08` is
also measured relative to the short edge; increase it if movement does not
reach full speed. `sprint_multiplier=2` multiplies that radius only while Shift
and net forward movement are held. Values 1..4 are accepted; the resulting
radius is capped at 0.5 of the short edge and coordinates stay within the image.
Existing profiles default to 2 without recalibration. Tune it to the game's
sprint threshold; this is a held joystick gesture, not a separate sprint-button
tap or an intentional sprint-lock release. Ubisoft describes sprint-lock as
[dragging past the joystick circle and releasing on the sprint icon](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2266-how-do-i-automatically-run-sprint-lock/?p=web).
Restart the viewer after editing the file.
The optional `sprint=x,y` endpoint takes priority over `sprint_multiplier`;
its y coordinate must be above `joystick`, and its x coordinate is not used
to steer. Optional `vault`, `rappel`, `secondary_gadget`, and `mount` fields use
ordinary target coordinates. A key targets a HUD slot, not a named item: B may activate
a different gadget on another operator. Separate profile files are appropriate
when the layout changes. F10 saves and applies a mapping without a restart.

The protocol supports **five contacts**. Movement and looking each reserve one;
three action buttons can be held together. Additional action presses are
rejected until a button is released and pressed again. Key repeat never starts
another touch.

Looking lifts and recenters its own contact at the edge of a small region while
preserving movement and buttons. Mouse vectors are split at region boundaries
and continued from the center in the same callback, preserving displacement
within contact-coordinate rounding. Work is limited to eight segment attempts
per callback; extreme deltas or outward motion from an edge calibration are
discarded beyond that bound and counted as `clipped_mouse_events`. No remainder
is queued for replay after the mouse stops. `look_resets` counts reanchors.
This is touch-based aiming: sensitivity, acceleration,
touch sampling and game behavior still apply. It is not guaranteed to behave
like native PC raw-mouse input.

A new joystick drag holds its center for 20 ms before moving. Back-to-back
down/move reports were merged on the tested phone, losing the initial drag.
Ordinary direction changes, looking and button presses add no such delay,
although they can wait behind the initial joystick press in the ordered input
queue. The joystick pays this setup cost on first movement and when movement
resumes after the idle contact has been lifted.

## Live qualification

Tested over Wi-Fi with an iPhone 15 / iOS 27 in Rainbow Six Mobile's shooting
range: concurrent movement/look/fire, primary/secondary selection, reload,
both lean directions, crouch, melee, grenade preparation and Escape while
controls were held. A five-contact combination of movement, look, fire, aim
and lean was exercised. These checks used an isolated Sway desktop; physical
mouse sensitivity remains a personal tuning setting. The contextual interact
mapping has not been qualified against an eligible interaction in that range.
The Safari diagnostic below has not been live-qualified on this phone.

Direction-change debugging reproduced lost initial joystick displacement with
back-to-back reports and verified movement after adding the initial 20 ms
spacing. Short WASD handoffs with look/lean held, neutral stopping, manual fire
and Escape were checked in the range. With both auto-shoot options off, hovering
over a training target did not fire or enter ADS. These are functional checks,
not a measurement of end-to-end input latency or a guarantee of PC-like aiming.

Shift sprint has offline coverage for both Shift keys, release/exit cleanup,
diagonals, rotations and preservation of simultaneous contacts. Its default
radius did not reliably activate sprint in the user's live match. In a subsequent
training check, calibrating an endpoint farther above the joystick activated
the running indicator and lowered the weapon; releasing movement restored neutral.
The endpoint is specific to that HUD and is not shipped as a default.
Vault/rappel/second-gadget/mount bindings have offline coverage; contextual
vault, mount and rappel still require gameplay verification at eligible surfaces.

Additional custom targets were exercised in training: ping placed a visible
marker; drone selection followed by fire deployed a drone and entered its view;
movement, jump, observation entry and exit responded. Drone scan was pressed
without an enemy in view, so enemy detection is not qualified. These targets
are configured with F10, not built-in drone actions. The tested private profile
uses Z for ping, 5 for drone selection, 7 for observation, 6 for observation exit,
J for drone jump and T for scan. The viewer does not switch control contexts:
combat fire/ADS/lean coordinates remain active in drone view. Do not treat this
as a complete PC drone control scheme or assume those keys suit another HUD.

Configurable key/mouse bindings have offline coverage for calibration and
persistence, alias ownership, wheel expiry, and release cleanup. Arbitrary new
HUD actions and individual mouse hardware still require live qualification.
Idle contact expiration passes timer and quick-handoff regression tests, but
the reported death/respawn issue still needs a live retest with this change.

## Diagnostic page

```sh
cargo run --example touch_page -- --bind 0.0.0.0 --port 46568
```

Open the printed port at this computer's LAN address in Safari on the phone.
The temporary page displays active/max contacts and logs synthetic touch
snapshots. Calibrate a **separate test profile** to that page. Check movement and
looking simultaneously, add buttons, release one while the others remain held,
then Escape: active contacts must return to zero. Stop the server with Ctrl+C.
Its endpoint accepts diagnostic events without authentication; use it only on
a trusted local network. Do not publish private gameplay screenshots or profiles
as test fixtures.
