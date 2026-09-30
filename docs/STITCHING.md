# Stitching games together

The goal is one runtime that can run content from several rewritten games:
CS:GO from this repo, MW2/BO1/MW3 from COD-Legacy-Mashup (IW4L), and more.
This page records what has been done so far to keep that possible.

## Shared foundations

| Decision | Why |
|---|---|
| **Bevy 0.19, glam 0.32** (same as IW4L) | Both runtimes can be plugins in one Bevy `App`. |
| **`content::ContentId` = `<origin>_<kind>_<name>`** | Same scheme as the COD Legacy Mashup spec (`t5_weapon_ak47`), so one catalog can hold `csgo_weapon_ak47` and `iw4_weapon_m4a1`. |
| **`content::GameOrigin`** includes `Csgo`, `T4`, `Iw4`, `T5`, `Iw5` and `T6` | The origin is metadata, not a wall. |
| **Normalised definitions** (`weapons::WeaponDefinition`) | An MW2 importer can emit the same struct, and gameplay code consumes one type. |
| **`client::CsgoPlugin`** is self-contained | It can be added to another game's `App`. |
| **Source units and axes stay inside the format crates** | Each game translates at its own boundary. |

## Plan

1. **Catalog crate.** Move `content` into a small shared crate both repos
   depend on (git dependency or workspace path). Add a `Catalog` that stores
   definitions from any origin.
2. **Weapon bridge.** Implement `From<WeaponDefinition>` into IW4L's
   universal `WeaponDefinition`, or the reverse. The two stat models differ:
   CS:GO uses inaccuracy/recoil seeds and damage with armor ratio, while IW4 uses
   spread cones and damage ranges. Keep an origin-specific "behaviour" enum
   so each weapon can keep its home game's feel.
3. **Movement profiles.** `movement::MoveConfig` is already data. An IW4
   profile (sprint, slide-less, 190 u/s) could live next to the CS:GO one, with
   the ruleset picking which applies.
4. **Maps.** A map loaded by either runtime produces a `CollisionWorld`-like
   trace interface plus renderable meshes. The `movement::TraceWorld` trait is
   the seam: IW4L's clipmap can implement it too.
5. **One app.** A launcher that adds `CsgoPlugin` and IW4L's plugins and
   switches maps and rulesets from a menu.
