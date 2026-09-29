# Rainbow Six PC controls versus the touch mapper

Research date: 2026-09-29. Implementation baseline: `0f701e5`.
This is a comparison and proposed test plan, not implemented functionality or
evidence of current in-game settings. No phone input was sent for this audit.
See [game controls](game-controls.md) for supported behavior and calibration.

## PC reference and existing coverage

Ubisoft's [September 2026 accessibility article](https://news.ubisoft.com/en-us/article/r2MFilRathMnz0nZ3zosN/rainbow-six-siege-accessibility-spotlight)
provides a current keyboard overview. Its screenshot selects **Custom**, so use
the displayed keys as examples, not verified factory defaults. PC also supports
multiple bindings and Press/Hold/Toggle interactions, including deployment
options. Those semantics matter as much as the letters assigned to actions.

Our mapper already supports WASD, Shift, Q/E, R, F, V, G, B, 1/2, C, Space, X,
and left/right mouse. These are hardcoded bindings. F10 calibrates a supported
binding's touchscreen target; it does not assign arbitrary keys or new actions.

The following additions would preserve the existing custom X/B assignments:

| Missing capability | Suggested binding | Required work in this mapper |
| --- | --- | --- |
| Slow walk | Alt | Calibrate a smaller joystick radius, distinct from normal movement and sprint |
| Dedicated prone | Ctrl | Expose and calibrate the Mobile prone button |
| Operator unique ability | 4 / middle mouse | Separate the ability from generic throwable/gadget slots |
| Ping | Z | Calibrate basic ping first; wheel selection needs a separate interaction |
| Observation tools | 5 | Add a camera/drone control mode with an explicit exit |
| Deploy drone | 6 | Calibrate deploy and subsequent drone controls |
| Ability mode | Configurable | Keep B as second gadget unless deliberately reassigned |
| Rappel stance / exit | Context-specific | Verify which Mobile HUD targets exist before assigning keys |
| Mouse side buttons / weapon wheel | Configurable | Handle these inputs; currently ignored/suppressed in game mode |

The first six suggested keys follow the published PC example; the implementation
column is our design proposal. The same PC overview shows B for ability mode,
C for rappel stance, and X for dropping the defuser. These need not replace the
user's accepted custom bindings. [Official keyboard overview](https://staticctf.ubisoft.com/J3yJr34U2pZ2Ieem48Dwy9uqj5PNUQTn/2PPFtDnUyf8KtThpwebeEl/22cafe3da588a0938436a410f52837d2/R6S_ControlOptions_MAK.jpg)

Do not copy an old chart's weapon fire-mode toggle: Ubisoft removed weapon
fire-mode switching in 2021, retaining switching for multi-mode abilities.
[High Calibre season notes](https://www.ubisoft.com/en-us/game/rainbow-six/siege/news-updates/seasons/highcalibre)

## Calibration comes before new bindings

A read-only check of the local saved profile found rappel and secondary-gadget
targets, but no vault target or explicit sprint endpoint. No coordinates or
private profile are included here. This is a dated local observation, not the
state of every installation or proof that either configured target works.

- **Space:** implemented, but inactive without its vault target. Use F10 →
  Space → the visible vault control at an eligible obstacle or in HUD setup.
- **Shift:** currently falls back to the sprint multiplier. Calibrate its
  forward endpoint and test actual running and stopping; do not assume greater
  displacement alone fixes sprint.
- **X/B:** targets exist, but still need context/operator-specific verification.

Ubisoft documents sprint-lock as dragging beyond the joystick circle and
**releasing on the sprint icon**. Our Shift gesture holds the displacement.
These are different behaviors. The source does not establish that a held
outward joystick cannot sprint; distinguish ordinary sprint from automatic
locking, and test cancellation before adding a lock gesture.
[Sprint-lock FAQ](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2266-how-do-i-automatically-run-sprint-lock/)

## Mobile settings audit

These are settings to inspect next, not settings changed or confirmed today.

1. **Independent leaning and firing.** Enable Hip Lean for Q/E without ADS.
   Disable automatic firing for a manual mouse baseline. Mobile exposes Hip
   Lean and Hip Fire Auto-Shoot independently; inspect the installed build's
   ADS auto-shoot option too. Keep the existing fixed-joystick and disabled
   double-tap setup described in [game controls](game-controls.md).
   [Toxic Fog notes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2388-patch-notes---1-5-toxic-fog/)
2. **Separate crouch and prone.** Enable the dedicated Prone Button in HUD
   customization before adding Ctrl. Holding the crouch target can otherwise
   invoke prone. Mobile deliberately exits ADS when entering or moving in
   prone; mapping cannot remove that game rule.
   [Sand Wraith announcement](https://www.ubisoft.com/en-sg/game/rainbow-six/mobile/news-updates/WoFDUQeYj8ClA2kL7hwbd)
3. **A predictable aiming curve.** Start controlled comparisons with Fixed
   touch acceleration, then measure equal-distance slow and fast mouse swipes.
   Ubisoft has patched fast-swipe limitations and prone sensitivity, so these
   are plausible game-side contributors. Fixed is a test baseline, not proof
   of raw or perfectly linear mouse input.
   [Sand Wraith hotfix](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2450-patch-notes---2-0-sand-wraith-hotfix/)
4. **Aim assist and gyro.** Compare with aim assist disabled to isolate the
   input response; keep whichever setting the player subsequently prefers.
   Aim assist is separate from auto-fire. Disable gyro during controlled mouse
   tests if phone movement would add input.
   [Aim-assist FAQ](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/1611-is-there-aim-assist-how-do-i-enable-it/),
   [Mobile 2.3 notes, including gyro changes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2563-patch-notes---2-3-concrete-impact/)
5. **Hold/toggle consistency.** Inspect ADS, lean, crouch, and gadget behavior
   individually. The mapper currently holds each target until key release and
   delegates behavior to the game. Do not layer a mapper toggle over a game
   toggle without an explicit, tested interaction model.

## Changes most likely to improve control feel

These recommendations are engineering inferences, not measured improvements.

**Configurable actions and gestures.** Separate key choice, target, and intended
behavior. Allow keyboard/mouse aliases with conflict reporting; releasing one
alias must not release an action still held by another. Prefer the game's own
hold/toggle options. Add tap/hold/toggle translation only where needed, with a
clear cancel path and no deferred presses firing after a mode change.

**Walk, normal movement, sprint.** Tune three useful joystick displacements
against observed game speeds. Preserve normalized diagonals and uninterrupted
direction changes. Avoid smoothing WASD transitions before measuring the cause:
a filter can delay the desired direction or release.

**Separate combat, drone, and gadget controls.** A combat fire coordinate may
mean something else in a drone HUD. Start with explicit mode selection and a
visible mode indicator; release old contacts on every transition. Add camera
cycling, scan, drone jump, exit, and contextual interaction only after observing
their actual targets. Mobile supports separate gadget-view HUD customization
and a ping communication wheel.
[Controls FAQ](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2265-what-are-the-controls/),
[Chain Reaction notes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2525-patch-notes---2-2-chain-reaction/)

**Name gadget operations accurately.** Unique ability, secondary gadget, ability
mode, equip, deploy/fire, and cancel are distinct. For example, Mobile's Zofia
launcher has impact and concussion modes. G/B target coordinates do not tell
us which item or interaction an operator exposes.
[Concrete Impact notes](https://ubisoft-mobile.helpshift.com/hc/en/45-rainbow-six-mobile/faq/2563-patch-notes---2-3-concrete-impact/)

**Measure hipfire and scopes separately.** Our mapper has one sensitivity
scalar. Compare physical mouse distance and observed rotation at different
speeds, scoped and unscoped, before adding another multiplier. Tune in-game
scope settings first where available. Do not infer actual ADS solely from RMB:
the game can enter or exit ADS through other actions. PC's sensitivity guide
also distinguishes matching rotation from matching apparent screen motion;
equal slider numbers across games are not a calibration.
[Ubisoft ADS guide](https://www.ubisoft.com/en-us/game/rainbow-six/siege/news-updates/3IMlDGlaRFgdvQNq3BOSFv/guide-to-ads-sensitivity-in-y5s3)

## Proposed verification order

1. Record game build, operator/HUD, input settings, connection, and profile.
   Calibrate Space and sprint; verify walk/run/sprint and release behavior.
2. Test independent lean/ADS/fire, crouch/prone transitions, and held F at valid
   interact/reinforce/defuse targets. Separate missing HUD targets from game
   rules and mapper failures.
3. Add and qualify prone, slow walk, ping, and operator ability individually.
   Then add explicit drone/gadget modes, including cancel/exit.
4. Stress direction handoffs while looking, leaning, aiming and firing; release
   inputs in different orders. The current implementation has five contacts:
   movement, look, and three action slots. Extra actions are rejected, not
   queued. Expose that limit when testing combinations.
5. Test death/respawn and context changes while keys are held. There is no game
   state detection. Verify Escape/F8, focus loss, and mode changes clear touches;
   do not describe the current idle cleanup as automatic respawn handling.
6. Compare equal-distance slow/fast mouse movements, long recentering sweeps,
   and scope changes. Use input traces and observed game response together;
   host dispatch time alone is not input-to-photon latency.

The backend translates input into touch gestures. We can improve consistency
and coverage, but matching native PC mouse behavior is a measurement target,
not a guarantee. Rendering/frame-pacing work is a separate axis; see the
dated [smoothness research](smoothness-research.md), rather than treating more
bindings as an FPS or latency improvement.
