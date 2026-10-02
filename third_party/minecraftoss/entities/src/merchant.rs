//! `MerchantMenu`, `MerchantContainer` and `MerchantResultSlot` (26.3): the
//! trading screen's two payment slots and result, over the player's
//! inventory. Every change to a payment slot re-reads the offers
//! (`updateSellItem`: the selected offer, or the first the payments satisfy,
//! either way round) and tells the merchant (`notifyTradeUpdated`, which is
//! when a villager says yes or no); taking the result pays for it and tells
//! the merchant of the trade (`notifyTrade`). The slot operations follow
//! `AbstractContainerMenu.doClick` and `Slot` closely, since how often the
//! offers are re-read decides which of those calls speak. Menu slots 3..30
//! are the inventory's 9..36, 30..39 its hotbar.
use crate::trading::MerchantOffer;
use minecraftoss_player::inventory::{Inventory, ItemStack};
use serde_json::Value;

/// What the menu asks of the one it trades with (`Merchant`).
pub trait Merchant {
    /// `getOffers`, made first if need be.
    fn offers(&mut self) -> Vec<MerchantOffer>;
    /// An item's maximum stack size.
    fn max_stack(&self, item: &str) -> i32;
    /// `notifyTradeUpdated`: whether a result shows now.
    fn trade_updated(&mut self, valid: bool);
    /// `notifyTrade` for the offer at `index`.
    fn trade(&mut self, index: usize);
}

/// `MerchantContainer.getMaxStackSize` (the `Container` default).
const CONTAINER_MAX: i32 = 99;

/// A player's open trading screen.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MerchantMenu {
    /// The villager traded with.
    pub villager: u64,
    pub payment: [Option<ItemStack>; 2],
    pub result: Option<ItemStack>,
    /// The offer picked in the list (`selectionHint`).
    pub selection_hint: i32,
    /// `activeOffer`, by index.
    pub active_offer: Option<usize>,
    /// The experience the shown result would bring (`futureXp`).
    pub future_xp: i32,
}

type View<'a> = Option<(&'a str, i32, Option<&'a Value>)>;

fn view(stack: &Option<ItemStack>) -> View<'_> {
    stack.as_ref().filter(|s| s.count > 0).map(|s| (s.id.as_str(), i32::from(s.count), s.components.as_ref()))
}

/// `ItemStack.isSameItemSameComponents`.
fn same(a: &ItemStack, b: &ItemStack) -> bool {
    a.id == b.id && a.components == b.components
}

/// A menu slot's inventory index.
fn inventory_index(menu_slot: usize) -> usize {
    if menu_slot < 30 { menu_slot - 3 + 9 } else { menu_slot - 30 }
}

/// `AbstractContainerMenu.moveItemStackTo` into the inventory's menu slots
/// `start..end`: onto matching stacks first, then into the first empty
/// slot, backwards or not. Whether anything moved.
fn move_item_stack_to(stack: &mut ItemStack, inventory: &mut Inventory, start: usize, end: usize, backwards: bool) -> bool {
    let order: Vec<usize> = if backwards { (start..end).rev().collect() } else { (start..end).collect() };
    let mut changed = false;
    // `isStackable`.
    if stack.max > 1 {
        for &menu_slot in &order {
            if stack.count == 0 {
                break;
            }
            let Some(target) = inventory.slots[inventory_index(menu_slot)].as_mut() else { continue };
            if !same(target, stack) {
                continue;
            }
            let total = i32::from(target.count) + i32::from(stack.count);
            let max = CONTAINER_MAX.min(i32::from(target.max));
            if total <= max {
                stack.count = 0;
                target.count = total as u8;
                changed = true;
            } else if i32::from(target.count) < max {
                stack.count -= (max - i32::from(target.count)) as u8;
                target.count = max as u8;
                changed = true;
            }
        }
    }
    if stack.count > 0 {
        for &menu_slot in &order {
            let slot = &mut inventory.slots[inventory_index(menu_slot)];
            if slot.is_none() {
                let max = CONTAINER_MAX.min(i32::from(stack.max));
                let moved = i32::from(stack.count).min(max) as u8;
                *slot = Some(ItemStack { count: moved, ..stack.clone() });
                stack.count -= moved;
                changed = true;
                break;
            }
        }
    }
    changed
}

impl MerchantMenu {
    pub fn new(villager: u64) -> Self {
        Self { villager, ..Self::default() }
    }

    /// `offer.assemble()`.
    fn assemble(offer: &MerchantOffer, merchant: &impl Merchant) -> ItemStack {
        let components = (!offer.sell.components.is_empty()).then(|| Value::Object(offer.sell.components.clone()));
        ItemStack { id: offer.sell.id.clone(), count: offer.sell.count as u8, max: merchant.max_stack(&offer.sell.id) as u8, components }
    }

    /// `MerchantOffers.getRecipeFor`: the selected offer if the payments
    /// satisfy it (a hint of zero is no hint), else the first they do.
    fn recipe_for(offers: &[MerchantOffer], a: View, b: View, hint: i32, merchant: &impl Merchant) -> Option<usize> {
        let fits = |o: &MerchantOffer| o.satisfied_by(a, b, merchant.max_stack(&o.buy.id));
        if hint > 0 && (hint as usize) < offers.len() {
            return fits(&offers[hint as usize]).then_some(hint as usize);
        }
        offers.iter().position(fits)
    }

    /// `MerchantContainer.updateSellItem`.
    pub fn update_sell_item(&mut self, merchant: &mut impl Merchant) {
        self.active_offer = None;
        let (a, b) = if view(&self.payment[0]).is_none() { (self.payment[1].clone(), None) } else { (self.payment[0].clone(), self.payment[1].clone()) };
        if view(&a).is_none() {
            self.result = None;
            self.future_xp = 0;
            return;
        }
        let offers = merchant.offers();
        if !offers.is_empty() {
            let mut offer = Self::recipe_for(&offers, view(&a), view(&b), self.selection_hint, merchant);
            if offer.is_none_or(|i| offers[i].out_of_stock()) {
                self.active_offer = offer;
                offer = Self::recipe_for(&offers, view(&b), view(&a), self.selection_hint, merchant);
            }
            match offer.filter(|&i| !offers[i].out_of_stock()) {
                Some(i) => {
                    self.active_offer = Some(i);
                    self.result = Some(Self::assemble(&offers[i], merchant));
                    self.future_xp = offers[i].xp;
                }
                None => {
                    self.result = None;
                    self.future_xp = 0;
                }
            }
        }
        merchant.trade_updated(self.result.is_some());
    }

    /// `MerchantContainer.setItem` on a payment slot: at most a stack, then
    /// the offers re-read.
    fn set_payment(&mut self, slot: usize, stack: Option<ItemStack>, merchant: &mut impl Merchant) {
        self.payment[slot] = stack.filter(|s| s.count > 0).map(|mut s| {
            s.count = s.count.min(CONTAINER_MAX.min(i32::from(s.max)) as u8);
            s
        });
        self.update_sell_item(merchant);
    }

    /// `Slot.setByPlayer` on a payment slot: set, then `setChanged`.
    fn set_payment_by_player(&mut self, slot: usize, stack: Option<ItemStack>, merchant: &mut impl Merchant) {
        self.set_payment(slot, stack, merchant);
        self.update_sell_item(merchant);
    }

    /// `setSelectionHint`.
    pub fn set_selection_hint(&mut self, hint: i32, merchant: &mut impl Merchant) {
        self.selection_hint = hint;
        self.update_sell_item(merchant);
    }

    /// `ServerboundSelectTradePacket`: the offer picked, then its costs
    /// moved from the inventory to the payment slots (`tryMoveItems`),
    /// the payments there first going back.
    pub fn select_trade(&mut self, index: i32, inventory: &mut Inventory, merchant: &mut impl Merchant) {
        self.set_selection_hint(index, merchant);
        let offers = merchant.offers();
        if index < 0 || index as usize >= offers.len() {
            return;
        }
        for slot in 0..2 {
            if let Some(mut old) = self.payment[slot].take() {
                if !move_item_stack_to(&mut old, inventory, 3, 39, true) {
                    self.payment[slot] = Some(old);
                    return;
                }
                self.set_payment(slot, Some(old), merchant);
            }
        }
        if self.payment[0].is_none() && self.payment[1].is_none() {
            let offer = offers[index as usize].clone();
            self.move_from_inventory(0, &offer.buy, inventory, merchant);
            if let Some(b) = &offer.buy_b {
                self.move_from_inventory(1, b, inventory, merchant);
            }
        }
    }

    /// `moveFromInventoryToPaymentSlot`: matching stacks, inventory first,
    /// until the payment slot holds a stack.
    fn move_from_inventory(&mut self, slot: usize, cost: &crate::trading::ItemCost, inventory: &mut Inventory, merchant: &mut impl Merchant) {
        for menu_slot in 3..39 {
            let index = inventory_index(menu_slot);
            let Some(item) = inventory.slots[index].clone() else { continue };
            if !cost.test(&item.id, item.components.as_ref()) {
                continue;
            }
            let current = self.payment[slot].clone();
            if current.as_ref().is_some_and(|c| !same(&item, c)) {
                continue;
            }
            let max = i32::from(item.max);
            let have = current.map_or(0, |c| i32::from(c.count));
            let moved = (max - have).min(i32::from(item.count));
            let paid = ItemStack { count: (have + moved) as u8, ..item.clone() };
            let left = i32::from(item.count) - moved;
            inventory.slots[index] = (left > 0).then(|| ItemStack { count: left as u8, ..item });
            let full = i32::from(paid.count) >= max;
            self.set_payment(slot, Some(paid), merchant);
            if full {
                break;
            }
        }
    }

    /// `MerchantResultSlot.mayPickup`.
    fn may_take_result(&mut self, merchant: &mut impl Merchant) -> bool {
        let Some(index) = self.active_offer else { return false };
        let offers = merchant.offers();
        let Some(offer) = offers.get(index) else { return false };
        let max = merchant.max_stack(&offer.buy.id);
        let (a, b) = (view(&self.payment[0]), view(&self.payment[1]));
        offer.satisfied_by(a, b, max) || offer.satisfied_by(b, a, max)
    }

    /// `MerchantResultSlot.onTake`: the payments pay for the active offer
    /// (`MerchantOffer.take`, either way round) and the merchant hears of
    /// the trade.
    fn take_result(&mut self, merchant: &mut impl Merchant) {
        let Some(index) = self.active_offer else { return };
        let offers = merchant.offers();
        let Some(offer) = offers.get(index).cloned() else { return };
        let max = merchant.max_stack(&offer.buy.id);
        let cost_a = offer.cost_a_count(max);
        let cost_b = offer.buy_b.as_ref().map_or(0, |b| b.count);
        let shrink = |stack: &mut Option<ItemStack>, by: i32| {
            if let Some(s) = stack.as_mut() {
                s.count = (i32::from(s.count) - by).max(0) as u8;
            }
        };
        let (mut a, mut b) = (self.payment[0].clone(), self.payment[1].clone());
        let paid = if offer.satisfied_by(view(&a), view(&b), max) {
            shrink(&mut a, cost_a);
            shrink(&mut b, cost_b);
            true
        } else if offer.satisfied_by(view(&b), view(&a), max) {
            shrink(&mut b, cost_a);
            shrink(&mut a, cost_b);
            true
        } else {
            false
        };
        if paid {
            merchant.trade(index);
            self.set_payment(0, a, merchant);
            self.set_payment(1, b, merchant);
        }
    }

    /// `quickMoveStack` on the result slot: the result into the inventory
    /// (hotbar first, from its end), the slot re-read, then paid for. What
    /// moved, if anything.
    fn quick_move_result_once(&mut self, inventory: &mut Inventory, merchant: &mut impl Merchant) -> Option<ItemStack> {
        let mut stack = self.result.clone()?;
        let clicked = stack.clone();
        if !move_item_stack_to(&mut stack, inventory, 3, 39, true) {
            return None;
        }
        // `setByPlayer(EMPTY)` or `setChanged`: the result is re-read
        // (what did not fit is lost to it, as in vanilla).
        self.result = (stack.count > 0).then(|| stack.clone());
        self.update_sell_item(merchant);
        if stack.count == clicked.count {
            return None;
        }
        self.take_result(merchant);
        Some(clicked)
    }

    /// A shift-click on the result: trades while the same item keeps
    /// coming (`doClick` with `QUICK_MOVE`).
    pub fn quick_move_result(&mut self, inventory: &mut Inventory, merchant: &mut impl Merchant) {
        if !self.may_take_result(merchant) {
            return;
        }
        let mut moved = self.quick_move_result_once(inventory, merchant);
        while let Some(item) = moved {
            if self.result.as_ref().is_none_or(|r| r.id != item.id) {
                break;
            }
            moved = self.quick_move_result_once(inventory, merchant);
        }
    }

    /// A click on the result (`doClick` with `PICKUP`): one trade's result
    /// onto an empty cursor, or onto a matching one with room for it.
    pub fn click_result(&mut self, inventory: &mut Inventory, merchant: &mut impl Merchant) {
        if let Some(clicked) = self.result.clone() {
            if self.may_take_result(merchant) {
                let room = match &inventory.cursor {
                    None => Some(None),
                    Some(carried) if same(carried, &clicked) => {
                        // `tryRemove` refuses what the cursor cannot hold (the
                        // result slot takes no items back).
                        let limit = i32::from(carried.max) - i32::from(carried.count);
                        (limit >= i32::from(clicked.count)).then(|| Some(carried.clone()))
                    }
                    Some(_) => None,
                };
                if let Some(carried) = room {
                    // The whole result goes; the emptied slot is re-read.
                    self.result = None;
                    self.update_sell_item(merchant);
                    inventory.cursor = Some(match carried {
                        None => clicked,
                        Some(mut c) => {
                            c.count += clicked.count;
                            c
                        }
                    });
                    self.take_result(merchant);
                }
            }
        }
        // `slot.setChanged()`.
        self.update_sell_item(merchant);
    }

    /// `Slot.safeInsert` on a payment slot: what the cursor keeps.
    fn safe_insert(&mut self, slot: usize, mut input: ItemStack, amount: i32, merchant: &mut impl Merchant) -> Option<ItemStack> {
        let current = self.payment[slot].clone();
        let have = current.as_ref().map_or(0, |c| i32::from(c.count));
        let transfer = amount.min(i32::from(input.count)).min(CONTAINER_MAX.min(i32::from(input.max)) - have);
        if transfer <= 0 {
            return Some(input);
        }
        match current {
            None => {
                self.set_payment_by_player(slot, Some(ItemStack { count: transfer as u8, ..input.clone() }), merchant);
                input.count -= transfer as u8;
            }
            Some(mut c) if same(&c, &input) => {
                input.count -= transfer as u8;
                c.count += transfer as u8;
                self.set_payment_by_player(slot, Some(c), merchant);
            }
            Some(_) => {}
        }
        (input.count > 0).then_some(input)
    }

    /// `Slot.tryRemove` on a payment slot.
    fn try_remove(&mut self, slot: usize, amount: i32, merchant: &mut impl Merchant) -> Option<ItemStack> {
        let current = self.payment[slot].clone()?;
        let taken = amount.min(i32::from(current.count));
        if taken <= 0 {
            return None;
        }
        let left = i32::from(current.count) - taken;
        // `ContainerHelper.removeItem`, then the offers re-read.
        self.payment[slot] = (left > 0).then(|| ItemStack { count: left as u8, ..current.clone() });
        self.update_sell_item(merchant);
        if left == 0 {
            self.set_payment_by_player(slot, None, merchant);
        }
        Some(ItemStack { count: taken as u8, ..current })
    }

    /// A click on a payment slot (`doClick` with `PICKUP`, or `QUICK_MOVE`
    /// with shift).
    pub fn click_payment(&mut self, slot: usize, right: bool, shift: bool, inventory: &mut Inventory, merchant: &mut impl Merchant) {
        if shift {
            // `quickMoveStack`: into the inventory, then the hotbar.
            let mut moved = self.quick_move_payment_once(slot, inventory, merchant);
            while let Some(item) = moved {
                if self.payment[slot].as_ref().is_none_or(|p| p.id != item.id) {
                    break;
                }
                moved = self.quick_move_payment_once(slot, inventory, merchant);
            }
            return;
        }
        let clicked = self.payment[slot].clone();
        let carried = inventory.cursor.take();
        match (clicked, carried) {
            (None, Some(carried)) => {
                let amount = if right { 1 } else { i32::from(carried.count) };
                inventory.cursor = self.safe_insert(slot, carried, amount, merchant);
            }
            (None, None) => {}
            (Some(clicked), None) => {
                let amount = if right { (i32::from(clicked.count) + 1) / 2 } else { i32::from(clicked.count) };
                if let Some(taken) = self.try_remove(slot, amount, merchant) {
                    inventory.cursor = Some(taken);
                    // `Slot.onTake`: `setChanged`.
                    self.update_sell_item(merchant);
                }
            }
            (Some(clicked), Some(carried)) => {
                if same(&clicked, &carried) {
                    let amount = if right { 1 } else { i32::from(carried.count) };
                    inventory.cursor = self.safe_insert(slot, carried, amount, merchant);
                } else if i32::from(carried.count) <= CONTAINER_MAX.min(i32::from(carried.max)) {
                    inventory.cursor = Some(clicked);
                    self.set_payment_by_player(slot, Some(carried), merchant);
                } else {
                    inventory.cursor = Some(carried);
                }
            }
        }
        // `slot.setChanged()`.
        self.update_sell_item(merchant);
    }

    fn quick_move_payment_once(&mut self, slot: usize, inventory: &mut Inventory, merchant: &mut impl Merchant) -> Option<ItemStack> {
        let mut stack = self.payment[slot].clone()?;
        let clicked = stack.clone();
        if !move_item_stack_to(&mut stack, inventory, 3, 39, false) {
            return None;
        }
        if stack.count == 0 {
            self.set_payment_by_player(slot, None, merchant);
        } else {
            self.payment[slot] = Some(stack.clone());
            self.update_sell_item(merchant);
        }
        if stack.count == clicked.count {
            return None;
        }
        // `Slot.onTake`: `setChanged`.
        self.update_sell_item(merchant);
        Some(clicked)
    }

    /// A shift-click on an inventory slot (by inventory index) while
    /// trading: between the inventory and the hotbar (`quickMoveStack`).
    pub fn quick_move_inventory(&mut self, index: usize, inventory: &mut Inventory) {
        if index >= 36 {
            return;
        }
        loop {
            let Some(mut stack) = inventory.slots[index].take() else { return };
            let clicked = stack.clone();
            let moved = if index >= 9 { move_item_stack_to(&mut stack, inventory, 30, 39, false) } else { move_item_stack_to(&mut stack, inventory, 3, 30, false) };
            inventory.slots[index] = (stack.count > 0).then(|| stack.clone());
            if !moved || stack.count == clicked.count {
                return;
            }
            if inventory.slots[index].as_ref().is_none_or(|s| s.id != clicked.id) {
                return;
            }
        }
    }

    /// `MerchantMenu.removed`: the cursor and the payments go back to the
    /// inventory (`placeItemBackInInventory`); what does not fit is
    /// returned to drop. The merchant stops trading in between.
    pub fn close(&mut self, inventory: &mut Inventory, selected: usize, stop: impl FnOnce()) -> Vec<ItemStack> {
        let mut drops = Vec::new();
        if let Some(carried) = inventory.cursor.take() {
            drops.extend(inventory.add_item(carried, selected));
        }
        stop();
        for slot in 0..2 {
            if let Some(stack) = self.payment[slot].take() {
                drops.extend(inventory.add_item(stack, selected));
            }
        }
        self.result = None;
        drops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trading::{ItemCost, TradeItem};

    /// A merchant with fixed offers, recording what it heard.
    #[derive(Default)]
    struct Fake {
        offers: Vec<MerchantOffer>,
        heard: Vec<bool>,
        trades: Vec<usize>,
    }

    impl Merchant for Fake {
        fn offers(&mut self) -> Vec<MerchantOffer> {
            self.offers.clone()
        }
        fn max_stack(&self, _: &str) -> i32 {
            64
        }
        fn trade_updated(&mut self, valid: bool) {
            self.heard.push(valid);
        }
        fn trade(&mut self, index: usize) {
            self.offers[index].uses += 1;
            self.trades.push(index);
        }
    }

    fn offer(buy: (&str, i32), sell: (&str, i32), max_uses: i32) -> MerchantOffer {
        MerchantOffer {
            buy: ItemCost { id: buy.0.into(), count: buy.1, components: None },
            buy_b: None,
            sell: TradeItem { id: sell.0.into(), count: sell.1, components: Default::default() },
            uses: 0,
            max_uses,
            reward_exp: true,
            special_price: 0,
            demand: 0,
            price_multiplier: 0.05,
            xp: 2,
        }
    }

    #[test]
    fn a_picked_trade_takes_its_payment_and_shift_click_trades_it_out() {
        let mut merchant = Fake { offers: vec![offer(("minecraft:wheat", 20), ("minecraft:emerald", 1), 2)], ..Fake::default() };
        let mut inventory = Inventory::default();
        inventory.slots[9] = Some(ItemStack::new("minecraft:wheat", 50));
        let mut menu = MerchantMenu::new(1);
        menu.select_trade(0, &mut inventory, &mut merchant);
        assert_eq!(menu.payment[0].as_ref().map(|s| s.count), Some(50), "all the wheat");
        assert!(inventory.slots[9].is_none());
        assert_eq!(menu.result.as_ref().map(|s| s.id.as_str()), Some("minecraft:emerald"));
        // Two uses: two trades, then the offer is out of stock.
        menu.quick_move_result(&mut inventory, &mut merchant);
        assert_eq!(merchant.trades, vec![0, 0]);
        assert_eq!(menu.payment[0].as_ref().map(|s| s.count), Some(10));
        assert!(menu.result.is_none(), "out of stock");
        let emeralds: u32 = inventory.slots.iter().flatten().filter(|s| s.id == "minecraft:emerald").map(|s| u32::from(s.count)).sum();
        assert_eq!(emeralds, 2);
        // Closing puts the rest back.
        let drops = menu.close(&mut inventory, 0, || {});
        assert!(drops.is_empty());
        assert_eq!(inventory.count("minecraft:wheat"), 10);
    }
}
