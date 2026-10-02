# Local collection

Open **Cases** on the side rail. **CASES**, **SOUVENIRS**, and **MY SKINS**
separate the collection. Open containers without keys; inspect, equip, apply
stickers, or scrape stickers from the reward/item screen.

The installed Legacy data currently supplies 39 weapon cases, 120 souvenir
packages, 1,732 weapon/glove finish combinations, and 6,711 sticker definitions.
Case rare pools cover 19 knife models and eight glove models, using the
corresponding original, Chroma, Gamma and glove-generation families.

## Console and terminal

Click **CONSOLE** or press backtick on the collection screen:

    give_case Revolution 5
    give_case "Paris 2023 Vertigo Souvenir Package" 2
    cases
    skins butterfly
    give_skin weapon_knife_butterfly 38 stattrak
    give_skin sporty_gloves 10018
    give_skin weapon_ak47 801 stattrak
    stickers crown
    give_sticker 1 4

Terminal equivalents use hyphens, e.g. **csr give-case Revolution 5**,
**csr give-skin weapon_ak47 801 stattrak**, **csr give-sticker 1 4**.
**csr skins fade** and **csr stickers crown** search installed names and IDs.
Counts must be 1–1000; ambiguous case names and unsupported weapon/paint/quality
combinations are rejected. StatTrak is restricted to case variants. Souvenir
grants must belong to an installed souvenir collection.

## Equipment and opening

Opening saves exactly one reward before the 6.4-second reel starts. Closing
the screen/game cannot discard it. Rare entries show the original gold mystery
art until the reveal. Original case sounds, artwork and Stratum fonts are used.

**EQUIP FINISH** equips the item on both teams. Rare knives replace the default
knife with the appropriate original model, draw/attack/inspect animations and
melee behavior. Gloves replace first-person hands while retaining team sleeves.
**USE DEFAULT** restores the slot without deleting owned items.

Finishes, quality, sticker slots and ownership travel with dropped weapons.
StatTrak counts enemy kills made by the original owner, including bots in this
offline game. Team kills, world damage and other players using the weapon do
not increase that owner's counter. Original numeric module/engraving meshes
display the count in first person and the inspection preview. Counts persist
to disk; a temporarily locked profile queues increments for retry.

Applying a sticker consumes one copy from sticker stock. The supported four
weapon slots use the original placement meshes. Each scrape adds 20% wear;
the fifth removes the sticker without refund. Souvenir packages award souvenir
quality and event-linked stickers, never StatTrak. Modern packages select the
tournament logo, two distinct team logos and the corresponding gold map sticker.
Older events use their available foil/gold kits and MVP signatures where present.

The main-menu character continues to use the original looping loadout animation.

## Saves and rendering

config/cosmetics.cfg stores local IDs, ownership, seeds, wear, counters,
souvenir event IDs, sticker stock and sticker slots. Version 1 profiles migrate
to version 2 on the next save. Saving uses an OS lock, reloads the current file
under the lock, syncs the temporary file, and renames it. Invalid profiles are
reported, never silently replaced. CSR_PROFILE_DIR selects an isolated profile.

All game content is read at runtime; no installed assets are copied into the
repository. The catalog is parsed once at startup. GPU paint materials are
cached per item/material. Original normal maps, paint patterns, material masks,
AO/curvature data and glove palette masks drive the finishes. Spray/airbrush
styles use authored position/surface textures for triplanar projection.

## Fidelity boundaries

This is a functioning local collection, not Steam inventory integration.
There are no purchases, market values, trading or keys. Local rarity weights
are blue:purple:pink:red:gold = 125:25:5:1:0.4, normalized to each container's
available tiers; lower collection tiers extend the same 5:1 progression.
Eligible rewards have a separate 10% StatTrak roll. These are local rules,
not a claim of reproducing Valve's server RNG.

Paint compositing, wear, environment reflections and holo/foil sticker effects
are not yet pixel-identical to Source. First-person stickers use original
geometry; world stickers use an approximate transform fitted from shared UV
landmarks. Counter modules and third-person gloves follow the weapon/player
skeletons. Souvenir teams are selected locally from the event's installed kits;
items do not claim the provenance of a real tournament match.

Clean-room finish behavior references:
[Valve's Anodized Airbrushed documentation](https://www.counter-strike.net/workshop/wf_anodizedairbrushed),
[Anodized Multicolored documentation](https://www.counter-strike.net/workshop/wf_anodizedmulticolored).

## Checks

    cargo test --workspace
    cargo run --offline --profile play -p launcher --example cosmeticscheck

The installed-content audit checks every container/finish icon and pattern,
case rare pools, all world sticker mappings, knife models, souvenir awards,
1,000 durable openings, save/reload/equipment, opening sounds and menu animation.
Temporary audit profiles and screenshot fixtures do not touch the user's save.

Fixtures: CSR_TEST_MENU=cases; CSR_TEST_CASE=contents/opening/reveal;
CSR_TEST_CASE_ID=4899; CSR_TEST_SKIN=801; CSR_TEST_QUALITY=stattrak;
CSR_TEST_STICKER=1. For rare finishes shared across models, select
CSR_TEST_SKIN_WEAPON=weapon_knife_butterfly alongside CSR_TEST_SKIN=38.
CSR_TEST_WEAPON sets the held weapon for screenshot runs.
