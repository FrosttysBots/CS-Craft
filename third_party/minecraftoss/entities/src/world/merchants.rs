//! Trading with villagers (26.3 `Villager.mobInteract`, `startTrading`,
//! `updateSpecialPrices`, `AbstractVillager.notifyTrade` and
//! `notifyTradeUpdated`, `Villager.rewardTradeXp` and
//! `increaseMerchantCareer`): a player's use opens the villager's offers
//! (a baby, or a villager with none, shakes its head) with prices set by
//! the player's reputation and Hero of the Village (and left so after, as
//! 26.3 no longer resets them when trading stops), each trade uses the offer, pays the villager experience (levelling it up
//! at once, with its next level's offers and regeneration) and drops an
//! experience orb, and the villager answers the payment slots with yes or
//! no. The screen itself is `crate::merchant`.
use super::*;
use crate::merchant::{Merchant, MerchantMenu};
use crate::trading::MerchantOffer;
use minecraftoss_player::inventory::{Inventory, ItemStack};

/// `VillagerData.NEXT_LEVEL_XP_THRESHOLDS`.
const LEVEL_XP: [i32; 5] = [0, 10, 70, 150, 250];

/// `VillagerData.canLevelUp`.
fn can_level_up(level: i32) -> bool {
    (1..5).contains(&level)
}

/// `VillagerData.getMaxXpPerLevel`: the experience its next level needs.
pub fn max_xp_for_level(level: i32) -> i32 {
    if can_level_up(level) { LEVEL_XP[level as usize] } else { 0 }
}

/// `VillagerData.getMinXpPerLevel`.
pub fn min_xp_for_level(level: i32) -> i32 {
    if can_level_up(level) { LEVEL_XP[level as usize - 1] } else { 0 }
}

/// What a player's use of a villager did (`InteractionResult`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VillagerUse {
    /// Not taken (`PASS`): another use may follow.
    Pass,
    /// Taken (`SUCCESS`): the trading screen opened, or a baby shook its head.
    Success,
    /// Taken with nothing to show (`CONSUME`): no offers.
    Consume,
}

/// The villager a menu trades with, as the menu asks of it.
struct VillagerMerchant<'a> {
    world: &'a mut EntityWorld,
    villager: u64,
}

impl Merchant for VillagerMerchant<'_> {
    fn offers(&mut self) -> Vec<MerchantOffer> {
        self.world.villager_offers(self.villager).map(<[MerchantOffer]>::to_vec).unwrap_or_default()
    }

    fn max_stack(&self, item: &str) -> i32 {
        self.world.trades.as_ref().map_or(64, |book| book.max_stack(item))
    }

    fn trade_updated(&mut self, valid: bool) {
        self.world.villager_trade_updated(self.villager, valid);
    }

    fn trade(&mut self, index: usize) {
        self.world.villager_notify_trade(self.villager, index);
    }
}

impl EntityWorld {
    /// An item's maximum stack size, from the trade data's item catalog.
    pub fn item_max_stack(&self, item: &str) -> i32 {
        self.trades.as_ref().map_or(64, |book| book.max_stack(item))
    }

    /// A player's Hero of the Village level (its amplifier), or none.
    pub fn set_player_hero(&mut self, player: u64, amplifier: Option<i32>) {
        match amplifier {
            Some(a) => {
                self.player_heroes.insert(player, a);
            }
            None => {
                self.player_heroes.remove(&player);
            }
        }
    }

    /// `makeSound` for a villager: its voice's pitch drawn from its random.
    fn villager_voice(&mut self, id: u64, event: &'static str) {
        let Some(entity) = self.villager_mut(id) else { return };
        let pitch = voice_pitch(&mut entity.random, entity.villager.age.baby());
        let at = entity.position();
        entity.voices.push((Voice::Event(event, 1.0, pitch), at));
    }

    /// `Villager.setUnhappy`: it shakes its head for 40 ticks and says no.
    fn villager_unhappy(&mut self, id: u64) {
        if let Some(entity) = self.villager_mut(id) {
            entity.unhappy = 40;
        }
        self.villager_voice(id, "entity.villager.no");
    }

    /// `Villager.mobInteract` by `player` with `held` in the hand used.
    pub fn villager_interact(&mut self, id: u64, player: u64, main_hand: bool, held: Option<&str>) -> VillagerUse {
        let Some(entity) = self.villagers.iter().find(|e| e.id == id) else { return VillagerUse::Pass };
        if held == Some("minecraft:villager_spawn_egg") || entity.villager.health <= 0.0 || entity.trading_player.is_some() || entity.sleeping.is_some() {
            return VillagerUse::Pass;
        }
        if entity.villager.age.baby() {
            self.villager_unhappy(id);
            return VillagerUse::Success;
        }
        let no_offers = self.villager_offers(id).is_none_or(<[MerchantOffer]>::is_empty);
        if main_hand && no_offers {
            self.villager_unhappy(id);
        }
        if no_offers {
            return VillagerUse::Consume;
        }
        // `startTrading`.
        self.villager_update_special_prices(id, player);
        if let Some(entity) = self.villager_mut(id) {
            entity.trading_player = Some(player);
        }
        self.merchant_menus.insert(player, MerchantMenu::new(id));
        VillagerUse::Success
    }

    /// `Villager.updateSpecialPrices`: each offer cheaper by the player's
    /// reputation times its price multiplier, and by Hero of the Village's
    /// share of its first cost (at least one).
    pub(super) fn villager_update_special_prices(&mut self, id: u64, player: u64) {
        let uuid = self.uuid_of(PLAYER_TARGET + player);
        let hero = self.player_heroes.get(&player).copied();
        let Some(entity) = self.villagers.iter_mut().find(|e| e.id == id) else { return };
        let reputation = entity.gossips.reputation(uuid);
        let Some(offers) = entity.offers.as_mut() else { return };
        offers.iter_mut().for_each(|o| o.special_price = 0);
        let hero = hero.map_or(0.0, |a| f64::from(0.3_f32 + 0.0625_f32 * a as f32));
        for offer in offers.iter_mut() {
            if reputation != 0 {
                let off = reputation as f32 * offer.price_multiplier;
                offer.special_price -= if off < (off as i32) as f32 { off as i32 - 1 } else { off as i32 };
            }
            if hero > 0.0 {
                let reduction = (hero * f64::from(offer.buy.count)).floor() as i32;
                offer.special_price -= reduction.max(1);
            }
        }
    }

    /// `AbstractVillager.notifyTradeUpdated`: yes or no, if it has not
    /// spoken in the last second.
    pub(super) fn villager_trade_updated(&mut self, id: u64, valid: bool) {
        let Some(entity) = self.villager_mut(id) else { return };
        if entity.ambient_sound_time > -80 + 20 {
            entity.ambient_sound_time = -80;
            self.villager_voice(id, if valid { "entity.villager.yes" } else { "entity.villager.no" });
        }
    }

    /// `AbstractVillager.notifyTrade` and `Villager.rewardTradeXp`: the offer
    /// used, the ambient clock reset, experience for the villager (and on
    /// reaching its level's threshold its next level at once, with that
    /// level's offers, prices renewed for its trader and ten seconds of
    /// regeneration), and an orb of 3 to 6 (8 more on a new level) unless
    /// the offer gives none.
    pub(super) fn villager_notify_trade(&mut self, id: u64, index: usize) {
        let book = self.trades.clone();
        let Some(entity) = self.villagers.iter_mut().find(|e| e.id == id) else { return };
        let Some(offer) = entity.offers.as_mut().and_then(|o| o.get_mut(index)) else { return };
        offer.uses += 1;
        let (xp, reward) = (offer.xp, offer.reward_exp);
        entity.ambient_sound_time = -80;
        let mut pop = 3 + entity.random.next_int(4) as i32;
        entity.villager.xp += xp;
        entity.last_traded_player = entity.trading_player;
        let level = entity.villager.level;
        if can_level_up(level) && entity.villager.xp >= max_xp_for_level(level) {
            // `increaseMerchantCareer`: the next level and its offers.
            entity.villager.level = level + 1;
            if let Some(book) = &book {
                let v = &entity.villager;
                let added = book.villager_offers(&v.kind, v.profession.id(), v.level, &mut self.trade_sequences);
                entity.offers.get_or_insert_with(Vec::new).extend(added);
            }
            let trader = entity.trading_player;
            entity.effects.add(crate::effects::EffectInstance::new(crate::effects::MobEffect::Regeneration, 200, 0));
            if let Some(player) = trader {
                self.villager_update_special_prices(id, player);
            }
            pop += 5;
        }
        if reward {
            if let Some(entity) = self.villagers.iter().find(|e| e.id == id) {
                let p = entity.villager.body.position;
                self.trade_experience.push((DVec3::new(p.x, p.y + 0.5, p.z), pop));
            }
        }
    }

    /// `AbstractVillager.stopTrading`: its trader gone. In 26.3 the special
    /// prices stay (and are saved) until the next `updateSpecialPrices`.
    pub(super) fn villager_stop_trading(&mut self, id: u64) {
        if let Some(entity) = self.villager_mut(id) {
            entity.trading_player = None;
        }
    }

    /// Experience orbs trades dropped since last taken: where, and worth
    /// how much.
    pub fn take_trade_experience(&mut self) -> Vec<(DVec3, i32)> {
        std::mem::take(&mut self.trade_experience)
    }

    /// A player's open trading screen.
    pub fn merchant_menu(&self, player: u64) -> Option<&MerchantMenu> {
        self.merchant_menus.get(&player)
    }

    /// `AbstractVillager.stillValid` for a player's screen: the villager
    /// still trades with them, lives, and is within their reach plus four
    /// (seven blocks from `eyes` to its box).
    pub fn merchant_still_valid(&self, player: u64, eyes: DVec3) -> bool {
        let Some(menu) = self.merchant_menus.get(&player) else { return false };
        let Some(e) = self.villagers.iter().find(|e| e.id == menu.villager) else { return false };
        if e.trading_player != Some(player) || e.villager.health <= 0.0 {
            return false;
        }
        let body = &e.villager.body;
        let half = f64::from(body.width) / 2.0;
        let (min, max) = (body.position - DVec3::new(half, 0.0, half), body.position + DVec3::new(half, f64::from(body.height), half));
        let outside = |v: f64, lo: f64, hi: f64| (lo - v).max(v - hi).max(0.0);
        let (dx, dy, dz) = (outside(eyes.x, min.x, max.x), outside(eyes.y, min.y, max.y), outside(eyes.z, min.z, max.z));
        dx * dx + dy * dy + dz * dz < (3.0 + 4.0) * (3.0 + 4.0)
    }

    /// Runs an operation on a player's trading screen with its villager.
    fn with_menu(&mut self, player: u64, op: impl FnOnce(&mut MerchantMenu, &mut VillagerMerchant)) {
        let Some(mut menu) = self.merchant_menus.remove(&player) else { return };
        let villager = menu.villager;
        op(&mut menu, &mut VillagerMerchant { world: self, villager });
        self.merchant_menus.insert(player, menu);
    }

    /// The trade picked in the list (`ServerboundSelectTradePacket`).
    pub fn merchant_select(&mut self, player: u64, index: i32, inventory: &mut Inventory) {
        self.with_menu(player, |menu, merchant| menu.select_trade(index, inventory, merchant));
    }

    /// A click on the result slot, shift-click or not.
    pub fn merchant_click_result(&mut self, player: u64, shift: bool, inventory: &mut Inventory) {
        self.with_menu(player, |menu, merchant| if shift { menu.quick_move_result(inventory, merchant) } else { menu.click_result(inventory, merchant) });
    }

    /// A click on a payment slot.
    pub fn merchant_click_payment(&mut self, player: u64, slot: usize, right: bool, shift: bool, inventory: &mut Inventory) {
        self.with_menu(player, |menu, merchant| menu.click_payment(slot, right, shift, inventory, merchant));
    }

    /// A shift-click on an inventory slot while trading.
    pub fn merchant_quick_move_inventory(&mut self, player: u64, index: usize, inventory: &mut Inventory) {
        if let Some(menu) = self.merchant_menus.get_mut(&player) {
            menu.quick_move_inventory(index, inventory);
        }
    }

    /// The trading screen closed (`MerchantMenu.removed`): the cursor and
    /// payments back to the inventory, the villager done trading. What did
    /// not fit, to drop.
    pub fn merchant_close(&mut self, player: u64, inventory: &mut Inventory, selected: usize) -> Vec<ItemStack> {
        let Some(mut menu) = self.merchant_menus.remove(&player) else { return Vec::new() };
        let villager = menu.villager;
        // `setTradingPlayer(null)` stops a villager that was trading.
        menu.close(inventory, selected, || {
            if self.villagers.iter().any(|e| e.id == villager && e.trading_player.is_some()) {
                self.villager_stop_trading(villager);
            }
        })
    }
}
