//! Card progression: card creation, packs, power-up (intensify), grade-up, selling, the
//! card book and the rewards for playing songs.
//!
//! Rules come from the game's own master data tables (CardData, CardBalanceData,
//! CardIntensifyData, CardSellData, CardStoreData, MusicData, CollectionAchievementData);
//! the numbers that weren't in any table are in server-data/progression.json
//! (games/<game>/progression.json). Design: games/ssomg/re/notes/06-progression.md (SSOMG; the other games use the same rules).
//!
//! Rewards go to the client as `RecentProvide` objects ({tag, item, quantity, context,
//! providableItemCode}), which the client applies to its own copy of the player; costs are
//! negative provides. `item` is a PaymentCategory.

use crate::assets::Assets;
use crate::state::{now_ms, State};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;

// PaymentCategory values (client enum).
pub const ITEM_PACK_NORMAL: i64 = 0;
// CardIntensifyResult, per entry of "intensifyResult" (IntensifyAssist reads 0 as success).
const RESULT_SUCCESS: i64 = 0;
const RESULT_FAILED: i64 = 1;
const RESULT_ABORT: i64 = 2;
pub const ITEM_PACK_PREMIUM: i64 = 1;
pub const ITEM_FREE_DIAMOND: i64 = 9;
pub const ITEM_FREE_RP: i64 = 10;
// PaymentCategory CARD_PACK_PRISM (any size) and CARD_PACK_PRISM_5/_10/_30 (SSGF).
const ITEM_PACK_PRISM: i64 = 64;
// PaymentCategory LIVE_THEME_CARD, VIDEO (moving photo), SOUND (voice): context lists of
// {code, objectID} / {code} / {code} (ProvideControl.AddLiveThemeCard / AddVideo / AddSound).
const ITEM_LIVE_THEME_CARD: i64 = 57;
const ITEM_LIVE_THEME_VIDEO: i64 = 58;
const ITEM_LIVE_THEME_SOUND: i64 = 59;
// LiveThemeItemState ON: an owned voice or moving photo, switched on.
const LIVE_THEME_ITEM_ON: i64 = 2;

pub const GRADE_R: i64 = 4;

struct CardDef {
    code: i64,
    artist: i64,
    group: i64,
}

#[derive(Clone, Copy)]
struct Balance {
    min_energy: i64,
    max_energy: i64,
    min_score: i64,
    max_score: i64,
}

pub struct Progression {
    cfg: Value,
    cards: Vec<CardDef>,
    /// Indices into `cards` of prism cards (CardData.prism set; SSGF only), the pool of "prism" packs
    /// and of a pack's prism slots (packs.<kind>.prismPercent); the other packs draw from `plain_cards`.
    prism_cards: Vec<usize>,
    plain_cards: Vec<usize>,
    by_group: HashMap<i64, Vec<usize>>,
    by_artist: HashMap<i64, Vec<usize>>,
    card_index: HashMap<i64, usize>,
    balance: Vec<Balance>,
    intensify: Vec<Map<String, Value>>,
    sell: Vec<i64>,
    store: HashMap<i64, Value>,
    rp_store: HashMap<i64, Value>,
    music_group: HashMap<i64, i64>,
    /// Songs per album (MusicData.groupData), hidden songs left out.
    group_songs: HashMap<i64, Vec<i64>>,
    /// Live theme (LiveThemeCardData.liveThemeID) -> its cards: (card code, voice, moving photo).
    live_themes: HashMap<i64, Vec<(i64, Option<i64>, Option<i64>)>>,
    achievements: HashMap<i64, Value>,
    /// Every lobby wallpaper (LobbyBgData) and artist pack (ArtistPackData): all owned offline.
    pub lobby_bgs: Vec<i64>,
    pub artist_packs: Vec<i64>,
    rng: Mutex<u64>,
}

fn table_rows(assets: &Assets, name: &str) -> Vec<Value> {
    assets.table(name).and_then(|t| t.as_array().cloned()).unwrap_or_default()
}

fn int(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).unwrap_or(0)
}

/// Level code used by CardBalanceData / CardIntensifyData rows: C1..S5 = 0..19, R1.. = 20..
pub fn level_code(grade: i64, level: i64) -> usize {
    let level = level.max(1);
    if grade >= GRADE_R { (20 + level - 1) as usize } else { (grade * 5 + level - 1).max(0) as usize }
}

/// Column name for a material card in CardIntensifyData, e.g. "B3" or "R12".
fn intensify_column(grade: i64, level: i64) -> String {
    let letter = ["C", "B", "A", "S", "R"].get(grade.clamp(0, 4) as usize).copied().unwrap_or("C");
    format!("{letter}{}", level.max(1))
}

impl Progression {
    pub fn load(assets: &Assets) -> Self {
        let cfg: Value = assets
            .data()
            .read("progression.json")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_else(|| json!({}));

        let mut cards = Vec::new();
        let mut prism_cards = Vec::new();
        let mut plain_cards = Vec::new();
        let (mut by_group, mut by_artist, mut card_index) = (HashMap::new(), HashMap::new(), HashMap::new());
        for c in table_rows(assets, "CardData") {
            if int(&c["material"]) != 0 {
                continue; // power-up material items, not collectible cards
            }
            let i = cards.len();
            let def = CardDef { code: int(&c["code"]), artist: int(&c["artistID"]), group: int(&c["groupID"]) };
            if int(&c["prism"]) != 0 {
                prism_cards.push(i);
            } else {
                plain_cards.push(i);
            }
            // Album and member pools (first-clear, milestone and grade-up cards) hold non-prism
            // cards; prism cards come from prism packs and prism slots only.
            if int(&c["prism"]) == 0 {
                by_group.entry(def.group).or_insert_with(Vec::new).push(i);
                by_artist.entry(def.artist).or_insert_with(Vec::new).push(i);
            }
            card_index.insert(def.code, i);
            cards.push(def);
        }

        let mut balance_rows = table_rows(assets, "CardBalanceData");
        balance_rows.sort_by_key(|r| int(&r["code"]));
        let balance = balance_rows
            .iter()
            .map(|r| Balance {
                min_energy: int(&r["minEnergy"]),
                max_energy: int(&r["maxEnergy"]),
                min_score: int(&r["minScore"]),
                max_score: int(&r["maxScore"]),
            })
            .collect();

        let mut intensify_rows = table_rows(assets, "CardIntensifyData");
        intensify_rows.sort_by_key(|r| int(&r["code"]));
        let intensify = intensify_rows.into_iter().filter_map(|r| r.as_object().cloned()).collect();

        let mut sell_rows = table_rows(assets, "CardSellData");
        sell_rows.sort_by_key(|r| int(&r["code"]));
        let sell = sell_rows.iter().map(|r| int(&r["price"])).collect();

        let store = table_rows(assets, "CardStoreData").into_iter().map(|r| (int(&r["code"]), r)).collect();
        let rp_store = table_rows(assets, "RhythmPointStoreData").into_iter().map(|r| (int(&r["code"]), r)).collect();
        let music_group =
            table_rows(assets, "MusicData").iter().map(|m| (int(&m["code"]), int(&m["groupData"]))).collect();
        let achievements =
            table_rows(assets, "CollectionAchievementData").into_iter().map(|r| (int(&r["code"]), r)).collect();
        let mut group_songs: HashMap<i64, Vec<i64>> = HashMap::new();
        for m in table_rows(assets, "MusicData") {
            if !m["isHidden"].as_bool().unwrap_or(false) {
                group_songs.entry(int(&m["groupData"])).or_default().push(int(&m["code"]));
            }
        }
        let mut live_themes: HashMap<i64, Vec<(i64, Option<i64>, Option<i64>)>> = HashMap::new();
        for c in table_rows(assets, "LiveThemeCardData") {
            live_themes.entry(int(&c["liveThemeID"])).or_default().push((
                int(&c["code"]),
                c["liveThemeSoundID"].as_i64(),
                c["liveThemeGifID"].as_i64(),
            ));
        }

        let seed = (now_ms() as u64) ^ ((std::process::id() as u64) << 32) | 1;
        Progression {
            cfg,
            cards,
            prism_cards,
            plain_cards,
            by_group,
            by_artist,
            card_index,
            balance,
            intensify,
            sell,
            store,
            rp_store,
            music_group,
            group_songs,
            live_themes,
            lobby_bgs: table_rows(assets, "LobbyBgData").iter().map(|r| int(&r["code"])).collect(),
            artist_packs: table_rows(assets, "ArtistPackData").iter().map(|r| int(&r["code"])).collect(),
            achievements,
            rng: Mutex::new(seed),
        }
    }

    pub fn card_count(&self) -> usize {
        self.cards.len()
    }

    pub fn cfg(&self) -> &Value {
        &self.cfg
    }

    // ------------------------------------------------------------------ randomness

    fn next(&self) -> u64 {
        let mut s = self.rng.lock().unwrap_or_else(|e| e.into_inner());
        // xorshift64*
        *s ^= *s >> 12;
        *s ^= *s << 25;
        *s ^= *s >> 27;
        s.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform float in [0, 1).
    fn unit(&self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&self, lo: i64, hi: i64) -> i64 {
        if hi <= lo { lo } else { lo + (self.next() % (hi - lo + 1) as u64) as i64 }
    }

    fn pick<'a, T>(&self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { items.get((self.next() % items.len() as u64) as usize) }
    }

    /// Weighted grade roll from {"grade": weight}, optionally only grades >= `min_grade`.
    fn roll_grade(&self, weights: &Value, min_grade: i64) -> i64 {
        let ws: Vec<(i64, f64)> = weights
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(g, w)| Some((g.parse::<i64>().ok()?, w.as_f64()?)))
                    .filter(|(g, _)| *g >= min_grade)
                    .collect()
            })
            .unwrap_or_default();
        let total: f64 = ws.iter().map(|(_, w)| w).sum();
        if total <= 0.0 {
            return min_grade.max(0);
        }
        let mut x = self.unit() * total;
        for (g, w) in &ws {
            if x < *w {
                return *g;
            }
            x -= w;
        }
        ws.last().map(|(g, _)| *g).unwrap_or(0)
    }

    // ------------------------------------------------------------------ cards

    fn roll_stats(&self, card: &mut Value) {
        let grade = int(&card["grade"]);
        let level = int(&card["level"]);
        if let Some(b) = self.balance.get(level_code(grade, level)).copied() {
            card["energy"] = json!(self.range(b.min_energy, b.max_energy));
            card["score"] = json!(self.range(b.min_score, b.max_score));
        }
    }

    /// Creates a card in the player's inventory (and card book) and returns it.
    pub fn new_card(&self, s: &mut State, card_id: i64, grade: i64, level: i64) -> Value {
        let object_id = s.save["nextCardObjectId"].as_i64().unwrap_or(1000);
        s.save["nextCardObjectId"] = json!(object_id + 1);
        let mut card = json!({
            "objectID": object_id, "cardID": card_id, "grade": grade, "level": level,
            "energy": 0, "score": 0, "isEquip": false, "isLock": false, "isPrism": false,
        });
        self.roll_stats(&mut card);
        if let Some(cards) = s.save["cards"].as_array_mut() {
            cards.push(card.clone());
        }
        register_card_book(s, card_id, grade);
        card
    }

    fn random_card_id(&self, pool: Option<&Vec<usize>>) -> i64 {
        let all: Vec<usize> = (0..self.cards.len()).collect();
        let pool = pool.filter(|p| !p.is_empty()).unwrap_or(&all);
        self.pick(pool).map(|&i| self.cards[i].code).unwrap_or(0)
    }

    /// Opens a pack ("normal" or "premium") of `size` cards, honoring the guarantees.
    pub fn open_pack(&self, s: &mut State, kind: &str, size: i64) -> Vec<Value> {
        // "prism" packs (SSGF) draw only prism cards; their odds default to premium packs'.
        let pcfg = match &self.cfg["packs"][kind] {
            Value::Null if kind == "prism" => &self.cfg["packs"]["premium"],
            c => c,
        };
        let weights = &pcfg["gradeWeights"];
        let prism_percent = pcfg["prismPercent"].as_f64().unwrap_or(0.0);
        let mut grades: Vec<i64> = (0..size).map(|_| self.roll_grade(weights, 0)).collect();
        let g = &pcfg["guarantee"][size.to_string()];
        let slot_mode = pcfg["guaranteeMode"].as_str() == Some("slot");
        if let (true, Some(grade), Some(count)) = (slot_mode, g["grade"].as_i64(), g["count"].as_i64()) {
            // "slot": `count` cards are exactly the guaranteed grade, the others rolled as usual
            // (the community model of SSGF's premium packs: e.g. 5-pack A = (1 + 0.21 * 4) / 5).
            let mut slots: Vec<usize> = (0..grades.len()).collect();
            for k in 0..(count.max(0) as usize).min(slots.len()) {
                let j = k + ((self.unit() * (slots.len() - k) as f64) as usize).min(slots.len() - k - 1);
                slots.swap(k, j);
                grades[slots[k]] = grade;
            }
        } else if let (Some(min_grade), Some(count)) = (g["grade"].as_i64(), g["count"].as_i64()) {
            let have = grades.iter().filter(|&&x| x >= min_grade).count() as i64;
            let mut missing = (count - have).max(0);
            // Upgrade the lowest slots until the guarantee holds.
            let mut order: Vec<usize> = (0..grades.len()).collect();
            order.sort_by_key(|&i| grades[i]);
            for i in order {
                if missing == 0 {
                    break;
                }
                if grades[i] < min_grade {
                    grades[i] = self.roll_grade(weights, min_grade).max(min_grade);
                    missing -= 1;
                }
            }
        }
        grades
            .into_iter()
            .map(|grade| {
                // Prism cards only come from prism packs and a pack's prism chance (SSGF's published
                // normal-pack odds: 1 % prism); games without prism cards draw from all cards.
                let prism = kind == "prism" || self.unit() * 100.0 < prism_percent;
                let pool = if prism { &self.prism_cards } else { &self.plain_cards };
                let id = self.random_card_id(Some(pool));
                self.new_card(s, id, grade, 1)
            })
            .collect()
    }

    pub fn pack_provide(&self, s: &mut State, kind: &str, size: i64, tag: &str) -> Value {
        let cards = self.open_pack(s, kind, size);
        let item = match (kind, size) {
            ("premium", _) => ITEM_PACK_PREMIUM,
            ("prism", 5) => ITEM_PACK_PRISM + 1,
            ("prism", 10) => ITEM_PACK_PRISM + 2,
            ("prism", 30) => ITEM_PACK_PRISM + 3,
            ("prism", _) => ITEM_PACK_PRISM,
            _ => ITEM_PACK_NORMAL,
        };
        provide(tag, item, cards.len() as i64, Value::Array(cards))
    }

    /// The tutorial pack(s) from the config's start.packs. The client shows them after the
    /// profile picker: TutorialAssist passes the rewards of tutorialStepComplete(1) to
    /// ProvideControl. Once per save. (The end-of-tutorial "Reset Nickname & Cards" reroll is
    /// patched out: skip-tutorial-proceed-dialog.)
    pub fn starter_provides(&self, s: &mut State) -> Vec<Value> {
        if s.save["claimed"]["starterPack"].as_bool().unwrap_or(false) {
            return Vec::new();
        }
        let packs = self.cfg["start"]["packs"].as_array().cloned().unwrap_or_default();
        let provides: Vec<Value> = packs
            .iter()
            .map(|p| {
                let size = p["size"].as_i64().unwrap_or(30);
                self.pack_provide(s, p["kind"].as_str().unwrap_or("normal"), size, "tutorialNormalCardPack30")
            })
            .collect();
        s.save["claimed"]["starterPack"] = json!(true);
        if s.save["player"]["leaderCard"]["c"].as_i64().unwrap_or(0) == 0 {
            if let Some(first) = s.save["cards"].get(0).map(|c| c["objectID"].as_i64().unwrap_or(0)) {
                s.set_leader(first);
            }
        }
        s.persist();
        provides
    }

    // ------------------------------------------------------------------ store

    /// Store.buyCardPack(code): CardStoreData row; paymentCategory 0 = RP, 1 = diamonds.
    pub fn buy_card_pack(&self, s: &mut State, code: i64) -> Result<Value, &'static str> {
        let row = self.store.get(&code).ok_or("unknown card store code")?;
        let price = int(&row["price"]);
        let qty = int(&row["quantity"]) + int(&row["bonusQuantity"]);
        let premium = int(&row["paymentCategory"]) == 1;
        let (balance, cost_item) = if premium {
            (total(s, "diamond", "freeDiamond"), ITEM_FREE_DIAMOND)
        } else {
            (total(s, "rhythmPoint", "freeRhythmPoint"), ITEM_FREE_RP)
        };
        if balance < price {
            return Err("not enough currency");
        }
        if premium { add_diamond(s, -price) } else { add_rp(s, -price) }
        let kind = if premium { "premium" } else { "normal" };
        let mut pack = self.pack_provide(s, kind, qty, "");
        pack["providableItemCode"] = row["product"].clone();
        s.persist();
        Ok(json!([pack, provide("", cost_item, -price, Value::Null)]))
    }

    /// Store.buyRhythmPoint(code): RhythmPointStoreData row, diamonds for RP (plus bonus).
    pub fn buy_rhythm_point(&self, s: &mut State, code: i64) -> Result<Value, &'static str> {
        let row = self.rp_store.get(&code).ok_or("unknown RP store code")?;
        let price = row["price"].as_f64().unwrap_or(0.0).round() as i64;
        let qty = int(&row["quantity"]) + int(&row["bonusQuantity"]);
        if total(s, "diamond", "freeDiamond") < price {
            return Err("not enough diamonds");
        }
        add_diamond(s, -price);
        add_rp(s, qty);
        s.persist();
        Ok(json!([provide("", ITEM_FREE_RP, qty, Value::Null), provide("", ITEM_FREE_DIAMOND, -price, Value::Null)]))
    }

    // ------------------------------------------------------------------ card operations

    fn max_level(&self, grade: i64) -> i64 {
        let gu = &self.cfg["gradeUp"];
        if grade >= GRADE_R { gu["maxRLevel"].as_i64().unwrap_or(50) } else { gu["maxLevel"].as_i64().unwrap_or(5) }
    }

    /// CardInventory.intensify: each material raises the target's level by one with the
    /// chance from CardIntensifyData[target level][material grade+level]; costs the summed
    /// "<column>Price" RP. Materials are consumed either way.
    pub fn intensify(&self, s: &mut State, target: i64, materials: &[i64], prob_rate: f64, price_rate: f64) -> Option<Value> {
        let mut card = find_card(s, target)?;
        let (grade, mut level) = (int(&card["grade"]), int(&card["level"]));
        let max = self.max_level(grade);
        let mut results = Vec::new();
        let mut price = 0.0;
        for &m in materials {
            let Some(mat) = find_card(s, m) else { results.push(RESULT_ABORT); continue };
            let col = intensify_column(int(&mat["grade"]), int(&mat["level"]));
            let row = self.intensify.get(level_code(grade, level));
            let chance = row.and_then(|r| r.get(&col)).and_then(Value::as_f64).unwrap_or(0.0);
            price += row.and_then(|r| r.get(&format!("{col}Price"))).and_then(Value::as_f64).unwrap_or(0.0);
            let ok = level < max && self.unit() * 100.0 < chance * prob_rate.max(0.0);
            if ok {
                level += 1;
            }
            results.push(if ok { RESULT_SUCCESS } else { RESULT_FAILED });
            remove_card(s, m);
        }
        let price = (price * price_rate.max(0.0)).round() as i64;
        card["level"] = json!(level);
        self.roll_stats(&mut card);
        replace_card(s, &card);
        add_rp(s, -price);
        s.persist();
        Some(json!({"intensifyResult": results, "card": card, "provide": provide("", ITEM_FREE_RP, -price, Value::Null)}))
    }

    /// CardInventory.upgrade (grade-up): two cards of the same member and grade (at max level
    /// if configured) become one card of the next grade, same member, level 1.
    pub fn grade_up(&self, s: &mut State, request: i64, target: i64, price_rate: f64) -> Result<Value, &'static str> {
        let a = find_card(s, request).ok_or("card not found")?;
        let b = find_card(s, target).ok_or("card not found")?;
        let grade = int(&a["grade"]);
        if grade >= GRADE_R || int(&b["grade"]) != grade {
            return Err("grades don't match");
        }
        let artist = |c: &Value| self.card_index.get(&int(&c["cardID"])).map(|&i| self.cards[i].artist);
        if artist(&a) != artist(&b) {
            return Err("different members");
        }
        if self.cfg["gradeUp"]["requireMaxLevel"].as_bool().unwrap_or(true) {
            let max = self.max_level(grade);
            if int(&a["level"]) < max || int(&b["level"]) < max {
                return Err("not at max level");
            }
        }
        let sell_price = self.sell.get(grade as usize).copied().unwrap_or(0);
        let price = ((2 * sell_price) as f64 * price_rate.max(0.0)).round() as i64;
        // A prism card grades up into a prism card of the same member.
        let member = artist(&a);
        let is_prism = |id: i64| self.card_index.get(&id).is_some_and(|&i| self.prism_cards.contains(&i));
        let prism_pool: Vec<usize> = if is_prism(int(&a["cardID"])) {
            self.prism_cards.iter().copied().filter(|&i| Some(self.cards[i].artist) == member).collect()
        } else {
            Vec::new()
        };
        let pool = if prism_pool.is_empty() { member.and_then(|ar| self.by_artist.get(&ar)) } else { Some(&prism_pool) };
        let new_id = self.random_card_id(pool);
        // The requesting card is upgraded in place (same objectID; the client looks it up by
        // that ID), the target card is used up.
        remove_card(s, target);
        let mut card = a;
        card["cardID"] = json!(new_id);
        card["grade"] = json!(grade + 1);
        card["level"] = json!(1);
        self.roll_stats(&mut card);
        replace_card(s, &card);
        register_card_book(s, new_id, grade + 1);
        add_rp(s, -price);
        s.persist();
        Ok(json!({"intensifyResult": [RESULT_SUCCESS], "card": card, "provide": provide("", ITEM_FREE_RP, -price, Value::Null)}))
    }

    /// CardInventory.sell / sells: CardSellData price per grade, as RP.
    pub fn sell(&self, s: &mut State, ids: &[i64]) -> i64 {
        let mut gain = 0;
        for &id in ids {
            if let Some(c) = find_card(s, id) {
                if c["isLock"].as_bool().unwrap_or(false) {
                    continue;
                }
                gain += self.sell.get(int(&c["grade"]) as usize).copied().unwrap_or(0);
                remove_card(s, id);
            }
        }
        add_rp(s, gain);
        s.persist();
        gain
    }

    // ------------------------------------------------------------------ card book

    pub fn card_book(&self, s: &State) -> Value {
        let list: Vec<Value> = s.save["cardBook"]
            .as_object()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        json!({
            "card": list, "themePointList": [],
            "cardRewardCode": s.save["claimed"]["cardBookCard"].as_i64().unwrap_or(-1),
            "themeRewardCode": s.save["claimed"]["cardBookTheme"].as_i64().unwrap_or(-1),
            "totalBuff": 0,
        })
    }

    /// CardBook.getReward(achievementCode, type): CollectionAchievementData reward
    /// (rewardItem 2 = diamonds, 4 = RP). Each achievement only once.
    pub fn card_book_reward(&self, s: &mut State, code: i64, kind: i64) -> Value {
        let Some(row) = self.achievements.get(&code) else { return json!([]) };
        let done = &mut s.save["claimed"]["achievements"];
        if !done.is_array() {
            *done = json!([]);
        }
        if done.as_array().is_some_and(|a| a.iter().any(|c| c.as_i64() == Some(code))) {
            return json!([]);
        }
        done.as_array_mut().map(|a| a.push(json!(code)));
        let field = if kind == 0 { "cardBookCard" } else { "cardBookTheme" };
        s.save["claimed"][field] = json!(code);
        let qty = int(&row["rewardQuantity"]);
        let p = match int(&row["rewardItem"]) {
            2 => {
                add_diamond(s, qty);
                provide("", ITEM_FREE_DIAMOND, qty, Value::Null)
            }
            _ => {
                add_rp(s, qty);
                provide("", ITEM_FREE_RP, qty, Value::Null)
            }
        };
        s.persist();
        json!([p])
    }

    // ------------------------------------------------------------------ playing

    pub fn song_rp(&self, difficulty: i64, score: i64) -> i64 {
        let c = &self.cfg["rpPerSong"];
        let base = c["base"].as_f64().unwrap_or(165.0);
        let rev = c["revision"].as_f64().unwrap_or(15000.0).max(1.0);
        let bonus = c["difficultyBonus"][difficulty.clamp(0, 2) as usize].as_f64().unwrap_or(18.0);
        let exp = c["exponent"].as_f64().unwrap_or(0.5);
        (base + bonus * (score.max(0) as f64 / rev).powf(exp)).round() as i64
    }

    fn grant_extra(&self, s: &mut State, spec: &Value, group: Option<i64>, tag: &str, out: &mut Vec<Value>) {
        if let Some(rp) = spec["rhythmPoint"].as_i64() {
            add_rp(s, rp);
            out.push(provide(tag, ITEM_FREE_RP, rp, Value::Null));
        }
        if let Some(d) = spec["diamond"].as_i64() {
            add_diamond(s, d);
            out.push(provide(tag, ITEM_FREE_DIAMOND, d, Value::Null));
        }
        for c in spec["cards"].as_array().cloned().unwrap_or_default() {
            let pool = group.and_then(|g| self.by_group.get(&g));
            let id = self.random_card_id(pool);
            let card = self.new_card(s, id, c["grade"].as_i64().unwrap_or(0), 1);
            out.push(provide(tag, ITEM_PACK_NORMAL, 1, json!([card])));
        }
        for p in spec["packs"].as_array().cloned().unwrap_or_default() {
            let kind = p["kind"].as_str().unwrap_or("normal");
            out.push(self.pack_provide(s, kind, p["size"].as_i64().unwrap_or(5), tag));
        }
    }

    /// Rewards for finishing a song: RP, first-clear card, song-count milestones, record
    /// ladder. Returns (reward provides, firstClearReward or null).
    pub fn stage_rewards(&self, s: &mut State, music: i64, difficulty: i64, score: i64) -> (Vec<Value>, Value) {
        let mut out = Vec::new();
        let rp = self.song_rp(difficulty, score);
        add_rp(s, rp);
        out.push(provide("music_play", ITEM_FREE_RP, rp, Value::Null));

        let group = self.music_group.get(&music).copied();
        let first_clear = Value::Null;
        // A save may still hold an undelivered first-clear card ("pendingGotcha"): hand it out now.
        let pending = s.save["pendingGotcha"].take();
        if pending.is_object() {
            let card = self.new_card(s, int(&pending["cardID"]), int(&pending["grade"]), int(&pending["level"]));
            out.push(provide("clearReward", ITEM_PACK_NORMAL, 1, json!([card])));
        }
        let key = format!("{music}_{difficulty}");
        if !s.save["firstClears"].as_object().is_some_and(|m| m.contains_key(&key)) {
            s.save["firstClears"][&key] = json!(now_ms());
            let grade = self.cfg["firstClear"]["gradeByDifficulty"][difficulty.clamp(0, 2) as usize].as_i64().unwrap_or(0);
            let id = self.random_card_id(group.and_then(|g| self.by_group.get(&g)));
            // Delivered as a "clearReward" pack: the result screen opens it with the normal card
            // popup. firstClearReward stays null, because a firstClearReward switches the client
            // to its reroll flow (CardRefreshControl), whose face-down card never opens offline.
            let card = self.new_card(s, id, grade, 1);
            out.push(provide("clearReward", ITEM_PACK_NORMAL, 1, json!([card])));
            // The result screen hangs (empty, after the card popup) when its rewards are only
            // card packs, so a first clear always comes with some RP too.
            let bonus = self.cfg["firstClear"]["rhythmPoint"].as_i64().unwrap_or(500).max(1);
            add_rp(s, bonus);
            out.push(provide("clearReward", ITEM_FREE_RP, bonus, Value::Null));
        }

        let played = s.save["counters"]["songsPlayed"].as_i64().unwrap_or(0) + 1;
        s.save["counters"]["songsPlayed"] = json!(played);
        for m in self.cfg["songMilestones"]["list"].as_array().cloned().unwrap_or_default() {
            let every = m["every"].as_i64().unwrap_or(0);
            if every > 0 && played % every == 0 {
                self.grant_extra(s, &m, group, "clearReward", &mut out);
            }
        }

        let songs = &mut s.save["counters"]["songsCleared"];
        if !songs.is_array() {
            *songs = json!([]);
        }
        if !songs.as_array().is_some_and(|a| a.iter().any(|x| x.as_i64() == Some(music))) {
            songs.as_array_mut().map(|a| a.push(json!(music)));
        }
        let cleared = s.save["counters"]["songsCleared"].as_array().map_or(0, |a| a.len()) as i64;
        let claimed = s.save["claimed"]["recordTier"].as_i64().unwrap_or(0);
        let tiers = self.cfg["recordLadder"]["tiers"].as_array().cloned().unwrap_or_default();
        for (i, t) in tiers.iter().enumerate() {
            let i = i as i64 + 1;
            if i > claimed && cleared >= t["songs"].as_i64().unwrap_or(i64::MAX) {
                self.grant_extra(s, t, None, "clearReward", &mut out);
                s.save["claimed"]["recordTier"] = json!(i);
            }
        }
        out.extend(self.live_theme_unlocks(s, "clearReward"));
        s.persist();
        (out, first_clear)
    }

    /// Albums whose every song has a record on difficulty `liveThemes.minDifficulty` or higher with at least
    /// `minStars` stars and `minScore` points (best stars and best score of that difficulty).
    fn complete_albums(&self, s: &State) -> usize {
        let c = &self.cfg["liveThemes"];
        let (min_diff, min_stars, min_score) = (
            c["minDifficulty"].as_i64().unwrap_or(1),
            c["minStars"].as_i64().unwrap_or(0),
            c["minScore"].as_i64().unwrap_or(0),
        );
        let records = &s.save["records"];
        let passed = |song: i64| {
            (min_diff..=2).any(|d| {
                let r = &records[format!("{song}_{d}")];
                r.is_object()
                    && r["stars"].as_i64().unwrap_or(0) >= min_stars
                    && r["score"].as_i64().unwrap_or(0) >= min_score
            })
        };
        self.group_songs.values().filter(|songs| songs.iter().all(|&m| passed(m))).count()
    }

    /// Live themes earned by completing albums (progression.json "liveThemes.unlocks": [{albums, theme}]),
    /// each once: all its cards with their voices and moving photos. Returns the reward provides.
    pub fn live_theme_unlocks(&self, s: &mut State, tag: &str) -> Vec<Value> {
        let mut out = Vec::new();
        let unlocks = self.cfg["liveThemes"]["unlocks"].as_array().cloned().unwrap_or_default();
        if unlocks.is_empty() {
            return out;
        }
        let albums = self.complete_albums(s) as i64;
        for u in unlocks {
            let theme = int(&u["theme"]);
            let done = s.save["claimed"]["liveThemes"].as_array().is_some_and(|a| a.iter().any(|t| t.as_i64() == Some(theme)));
            if done || albums < u["albums"].as_i64().unwrap_or(i64::MAX) {
                continue;
            }
            let Some(cards) = self.live_themes.get(&theme) else { continue };
            let lt = &mut s.save["liveThemes"];
            if !lt.is_object() {
                *lt = json!({"cards": [], "sounds": [], "gifs": [], "leader": null, "nextObjectId": 1});
            }
            let (mut card_ctx, mut sound_ctx, mut gif_ctx) = (Vec::new(), Vec::new(), Vec::new());
            for &(code, sound, gif) in cards {
                let id = lt["nextObjectId"].as_i64().unwrap_or(1);
                lt["nextObjectId"] = json!(id + 1);
                lt["cards"].as_array_mut().map(|a| {
                    a.push(json!({
                        "objectID": id, "liveThemeCard": code, "isEquip": false, "isBackground": false,
                        "liveThemeSound": sound.map(|_| LIVE_THEME_ITEM_ON), "liveThemeGif": gif.map(|_| LIVE_THEME_ITEM_ON),
                    }))
                });
                card_ctx.push(json!({"code": code, "objectID": id}));
                for (list, ctx, item) in [("sounds", &mut sound_ctx, sound), ("gifs", &mut gif_ctx, gif)] {
                    if let Some(code) = item {
                        lt[list].as_array_mut().map(|a| a.push(json!(code)));
                        ctx.push(json!({"code": code}));
                    }
                }
            }
            out.push(provide(tag, ITEM_LIVE_THEME_CARD, card_ctx.len() as i64, json!(card_ctx)));
            if !gif_ctx.is_empty() {
                out.push(provide(tag, ITEM_LIVE_THEME_VIDEO, gif_ctx.len() as i64, json!(gif_ctx)));
            }
            if !sound_ctx.is_empty() {
                out.push(provide(tag, ITEM_LIVE_THEME_SOUND, sound_ctx.len() as i64, json!(sound_ctx)));
            }
            let claimed = &mut s.save["claimed"]["liveThemes"];
            if !claimed.is_array() {
                *claimed = json!([]);
            }
            claimed.as_array_mut().map(|a| a.push(json!(theme)));
        }
        out
    }

    /// LiveThemeBook.getList: owned live theme cards, voices, moving photos and the main theme.
    pub fn live_theme_list(&self, s: &State) -> Value {
        let lt = &s.save["liveThemes"];
        json!({
            "liveThemeCards": lt["cards"].as_array().cloned().unwrap_or_default(),
            "liveThemeSounds": lt["sounds"].as_array().cloned().unwrap_or_default(),
            "liveThemeGifs": lt["gifs"].as_array().cloned().unwrap_or_default(),
            "liveThemeLeader": lt["leader"].clone(),
        })
    }
}

// ---------------------------------------------------------------------- helpers

pub fn provide(tag: &str, item: i64, quantity: i64, context: Value) -> Value {
    json!({"tag": tag, "item": item, "quantity": quantity, "context": context, "providableItemCode": 0})
}

fn total(s: &State, paid: &str, free: &str) -> i64 {
    s.save["player"][paid].as_i64().unwrap_or(0) + s.save["player"][free].as_i64().unwrap_or(0)
}

/// Adds (or, if negative, spends) free RP; spending takes free first, then paid.
pub fn add_rp(s: &mut State, n: i64) {
    add_currency(s, "freeRhythmPoint", "rhythmPoint", n);
}

pub fn add_diamond(s: &mut State, n: i64) {
    add_currency(s, "freeDiamond", "diamond", n);
}

fn add_currency(s: &mut State, free: &str, paid: &str, n: i64) {
    let p = &mut s.save["player"];
    let f = p[free].as_i64().unwrap_or(0);
    if n >= 0 || f + n >= 0 {
        p[free] = json!(f + n);
    } else {
        p[free] = json!(0);
        let rest = f + n; // negative remainder
        p[paid] = json!((p[paid].as_i64().unwrap_or(0) + rest).max(0));
    }
}

pub fn find_card(s: &State, object_id: i64) -> Option<Value> {
    s.save["cards"].as_array()?.iter().find(|c| c["objectID"].as_i64() == Some(object_id)).cloned()
}

fn replace_card(s: &mut State, card: &Value) {
    if let Some(cards) = s.save["cards"].as_array_mut() {
        if let Some(c) = cards.iter_mut().find(|c| c["objectID"] == card["objectID"]) {
            *c = card.clone();
        }
    }
}

pub fn remove_card(s: &mut State, object_id: i64) {
    if let Some(cards) = s.save["cards"].as_array_mut() {
        cards.retain(|c| c["objectID"].as_i64() != Some(object_id));
    }
}

pub fn set_card_flag(s: &mut State, ids: &[i64], field: &str, value: bool) {
    if let Some(cards) = s.save["cards"].as_array_mut() {
        for c in cards.iter_mut() {
            if ids.contains(&c["objectID"].as_i64().unwrap_or(-1)) {
                c[field] = json!(value);
            }
        }
    }
    s.persist();
}

/// Card book: best grade per card ID.
pub fn register_card_book(s: &mut State, card_id: i64, grade: i64) {
    let key = card_id.to_string();
    let book = &mut s.save["cardBook"];
    if !book.is_object() {
        *book = json!({});
    }
    let prev = book[&key]["grade"].as_i64().unwrap_or(-1);
    if grade > prev {
        let created = book[&key]["createdAt"].as_i64().unwrap_or_else(now_ms);
        book[&key] = json!({"cardID": card_id, "grade": grade, "createdAt": created});
    }
}
