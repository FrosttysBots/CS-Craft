//! Competitive round rules: phases, win conditions, economy and buying.
//!
//! The numbers come from the game's own `cfg/gamemode_competitive.cfg`
//! (`mp_freezetime`, `cash_team_loser_bonus`, ...) when it is available, so
//! the rules follow whatever the install ships. [`RulesConfig::default`]
//! holds the same values for tests and installs without the file.
//! `mp_c4timer` and `mp_round_restart_delay` are not in that file; their
//! defaults are the game's documented defaults (40 s and 7 s).

use crate::Team;
use std::collections::HashMap;

/// Which rule set the world runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    /// Respawn after a short delay with a fixed loadout, no objectives.
    Deathmatch,
    /// Solo persistent world: no rounds, time limit, objectives or CS bots.
    Special,
    /// Rounds, money, buying and the bomb.
    Competitive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RulesConfig {
    pub freeze_time: f32,
    /// Seconds after the freeze time ends during which buying is allowed.
    pub buy_time: f32,
    /// Round length after the freeze time, seconds.
    pub round_time: f32,
    pub restart_delay: f32,
    pub c4_timer: f32,
    pub plant_time: f32,
    pub defuse_time: f32,
    pub defuse_time_kit: f32,
    pub start_money: i32,
    pub max_money: i32,
    pub max_rounds: u32,
    pub halftime: bool,
    pub can_clinch: bool,
    pub starting_losses: u32,
    pub loser_bonus: i32,
    pub loser_bonus_consecutive: i32,
    pub win_elimination: i32,
    pub win_bomb: i32,
    pub win_defuse: i32,
    pub win_time: i32,
    pub planted_but_defused: i32,
    pub player_planted: i32,
    pub player_defused: i32,
    pub kill_award_factor: f32,
    pub t_default_secondary: String,
    pub ct_default_secondary: String,
    pub t_default_melee: String,
    pub ct_default_melee: String,
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            freeze_time: 15.0,
            buy_time: 20.0,
            round_time: 1.92 * 60.0,
            restart_delay: 7.0,
            c4_timer: 40.0,
            plant_time: 3.0,
            defuse_time: 10.0,
            defuse_time_kit: 5.0,
            start_money: 800,
            max_money: 16000,
            max_rounds: 30,
            halftime: true,
            can_clinch: true,
            starting_losses: 1,
            loser_bonus: 1400,
            loser_bonus_consecutive: 500,
            win_elimination: 3250,
            win_bomb: 3500,
            win_defuse: 3500,
            win_time: 3250,
            planted_but_defused: 800,
            player_planted: 300,
            player_defused: 300,
            kill_award_factor: 1.0,
            t_default_secondary: "weapon_glock".into(),
            ct_default_secondary: "weapon_hkp2000".into(),
            t_default_melee: "weapon_knife_t".into(),
            ct_default_melee: "weapon_knife".into(),
        }
    }
}

/// Parses a `.cfg` file of `name value // comment` lines.
pub fn parse_cfg(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or("").trim();
        let mut parts = line.splitn(2, char::is_whitespace);
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else { continue };
        let value = value.trim().trim_matches('"').to_string();
        out.insert(name.to_ascii_lowercase(), value);
    }
    out
}

impl RulesConfig {
    /// Overrides defaults with any matching cvars.
    pub fn from_cvars(cvars: &HashMap<String, String>) -> Self {
        let mut c = Self::default();
        let f = |k: &str| cvars.get(k).and_then(|v| v.parse::<f32>().ok());
        let i = |k: &str| f(k).map(|v| v as i32);
        let s = |k: &str| cvars.get(k).filter(|v| !v.is_empty()).cloned();
        if let Some(v) = f("mp_freezetime") { c.freeze_time = v; }
        if let Some(v) = f("mp_buytime") { c.buy_time = v; }
        if let Some(v) = f("mp_roundtime_defuse").filter(|v| *v > 0.0).or_else(|| f("mp_roundtime")) { c.round_time = v * 60.0; }
        if let Some(v) = f("mp_round_restart_delay") { c.restart_delay = v; }
        if let Some(v) = f("mp_c4timer") { c.c4_timer = v; }
        if let Some(v) = i("mp_startmoney") { c.start_money = v; }
        if let Some(v) = i("mp_maxmoney") { c.max_money = v; }
        if let Some(v) = i("mp_maxrounds") { c.max_rounds = v.max(1) as u32; }
        if let Some(v) = i("mp_halftime") { c.halftime = v != 0; }
        if let Some(v) = i("mp_match_can_clinch") { c.can_clinch = v != 0; }
        if let Some(v) = i("mp_starting_losses") { c.starting_losses = v.max(0) as u32; }
        if let Some(v) = i("cash_team_loser_bonus") { c.loser_bonus = v; }
        if let Some(v) = i("cash_team_loser_bonus_consecutive_rounds") { c.loser_bonus_consecutive = v; }
        if let Some(v) = i("cash_team_elimination_bomb_map") { c.win_elimination = v; }
        if let Some(v) = i("cash_team_terrorist_win_bomb") { c.win_bomb = v; }
        if let Some(v) = i("cash_team_win_by_defusing_bomb") { c.win_defuse = v; }
        if let Some(v) = i("cash_team_win_by_time_running_out_bomb") { c.win_time = v; }
        if let Some(v) = i("cash_team_planted_bomb_but_defused") { c.planted_but_defused = v; }
        if let Some(v) = i("cash_player_bomb_planted") { c.player_planted = v; }
        if let Some(v) = i("cash_player_bomb_defused") { c.player_defused = v; }
        if let Some(v) = f("cash_player_killed_enemy_factor") { c.kill_award_factor = v; }
        if let Some(v) = s("mp_t_default_secondary") { c.t_default_secondary = v; }
        if let Some(v) = s("mp_ct_default_secondary") { c.ct_default_secondary = v; }
        // The cfg names `weapon_knife` for both sides; the T knife is the
        // separate `weapon_knife_t` item, so only override the CT one.
        if let Some(v) = s("mp_ct_default_melee") { c.ct_default_melee = v; }
        c
    }

    /// Loss bonus for a team's `losses` counter (after counting this loss).
    pub fn loss_bonus(&self, losses: u32) -> i32 {
        let steps = losses.saturating_sub(1).min(4) as i32;
        self.loser_bonus + self.loser_bonus_consecutive * steps
    }

    pub fn defuse_duration(&self, kit: bool) -> f32 {
        if kit { self.defuse_time_kit } else { self.defuse_time }
    }

    /// Rounds needed to win the match.
    pub fn rounds_to_win(&self) -> u32 {
        self.max_rounds / 2 + 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Players are frozen at spawn and may buy.
    Freeze,
    /// The round is being played.
    Live,
    /// The round has been decided; players may still move and shoot.
    Over,
    /// The match is over; a new one starts after a delay.
    MatchOver,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundEndReason {
    TargetBombed,
    BombDefused,
    /// All Terrorists dead (and no bomb ticking).
    TerroristsEliminated,
    CounterTerroristsEliminated,
    /// The round timer ran out without a plant.
    TargetSaved,
    Draw,
}

impl RoundEndReason {
    pub fn winner(self) -> Option<Team> {
        match self {
            Self::TargetBombed | Self::CounterTerroristsEliminated => Some(Team::Terrorist),
            Self::BombDefused | Self::TerroristsEliminated | Self::TargetSaved => Some(Team::CounterTerrorist),
            Self::Draw => None,
        }
    }

    /// The announcement the game shows for this result.
    pub fn message(self) -> &'static str {
        match self {
            Self::TargetBombed => "Target Bombed",
            Self::BombDefused => "Bomb Defused",
            Self::TerroristsEliminated => "Counter-Terrorists Win",
            Self::CounterTerroristsEliminated => "Terrorists Win",
            Self::TargetSaved => "Target Saved",
            Self::Draw => "Round Draw",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoundState {
    pub phase: Phase,
    /// 1-based round number within the match.
    pub round: u32,
    /// Sim time the current phase began.
    pub phase_start: f32,
    /// Sim time the live part of this round began (end of freeze time).
    pub live_start: f32,
    pub score_t: u32,
    pub score_ct: u32,
    pub losses_t: u32,
    pub losses_ct: u32,
    pub last_result: Option<RoundEndReason>,
    pub match_winner: Option<Team>,
    /// The bomb was planted at some point this round.
    pub bomb_planted: bool,
}

impl RoundState {
    pub fn new(cfg: &RulesConfig, now: f32) -> Self {
        Self {
            phase: Phase::Freeze,
            round: 1,
            phase_start: now,
            live_start: now + cfg.freeze_time,
            score_t: 0,
            score_ct: 0,
            losses_t: cfg.starting_losses,
            losses_ct: cfg.starting_losses,
            last_result: None,
            match_winner: None,
            bomb_planted: false,
        }
    }

    pub fn score(&self, team: Team) -> u32 {
        match team {
            Team::Terrorist => self.score_t,
            Team::CounterTerrorist => self.score_ct,
            Team::Spectator => 0,
        }
    }

    /// Seconds left in the current phase's countdown (freeze time, round
    /// timer, or the delay before the next round).
    pub fn time_left(&self, cfg: &RulesConfig, now: f32) -> f32 {
        let end = match self.phase {
            Phase::Freeze => self.phase_start + cfg.freeze_time,
            Phase::Live => self.live_start + cfg.round_time,
            Phase::Over => self.phase_start + cfg.restart_delay,
            Phase::MatchOver => self.phase_start + cfg.restart_delay * 2.0,
        };
        (end - now).max(0.0)
    }

    pub fn can_buy(&self, cfg: &RulesConfig, now: f32) -> bool {
        match self.phase {
            Phase::Freeze => true,
            Phase::Live => now < self.live_start + cfg.buy_time,
            _ => false,
        }
    }

    /// Pistol rounds: the first round of each half.
    pub fn is_pistol_round(&self, cfg: &RulesConfig) -> bool {
        self.round == 1 || (cfg.halftime && self.round == cfg.max_rounds / 2 + 1)
    }
}

/// A buy request, carried in [`crate::TickInput`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Purchase {
    #[default]
    None,
    /// A weapon by item definition index.
    Weapon(u32),
    Kevlar,
    KevlarHelmet,
    DefuseKit,
}

/// Equipment prices (`item_kevlar`, `item_assaultsuit`, `item_defuser`).
pub const KEVLAR_PRICE: i32 = 650;
pub const HELMET_PRICE: i32 = 350;
pub const DEFUSE_KIT_PRICE: i32 = 400;

/// Why a purchase was refused (for UI feedback).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyError {
    NotAllowedNow,
    NotInBuyZone,
    WrongTeam,
    AlreadyOwned,
    NotEnoughMoney,
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_parsing_and_overrides() {
        let cvars = parse_cfg(
            "mp_freezetime\t\t\t15\nmp_roundtime   1.92 // minutes\ncash_team_loser_bonus 1400\nmp_ct_default_secondary\tweapon_hkp2000\nmp_t_default_primary \"\"\n",
        );
        let c = RulesConfig::from_cvars(&cvars);
        assert_eq!(c.freeze_time, 15.0);
        assert!((c.round_time - 115.2).abs() < 1e-3);
        assert_eq!(c.ct_default_secondary, "weapon_hkp2000");
        assert_eq!(cvars.get("mp_t_default_primary").map(String::as_str), Some(""));
    }

    #[test]
    fn loss_bonus_ladder() {
        let c = RulesConfig::default();
        // Starting losses of 1: the first loss pays 1900, capping at 3400.
        assert_eq!(c.loss_bonus(2), 1900);
        assert_eq!(c.loss_bonus(1), 1400);
        assert_eq!(c.loss_bonus(5), 3400);
        assert_eq!(c.loss_bonus(9), 3400);
    }

    #[test]
    fn buy_window() {
        let c = RulesConfig::default();
        let mut r = RoundState::new(&c, 0.0);
        assert!(r.can_buy(&c, 5.0));
        r.phase = Phase::Live;
        r.live_start = 15.0;
        assert!(r.can_buy(&c, 30.0));
        assert!(!r.can_buy(&c, 36.0));
    }
}
