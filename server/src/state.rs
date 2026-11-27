//! The single offline player (named in progression.json, "start.nickname") and everything the server remembers about them,
//! stored as one JSON file.

use serde_json::{json, Map, Value};
use crate::progression::Progression;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Name used when progression.json doesn't give one.
pub const NICKNAME: &str = "Player";
pub const USER_OBJECT_ID: i64 = 1;
const FREE_NICKNAME_CHANGES: i64 = 99;
/// 2 = card progression (start with the tutorial pack); 1 had every card unlocked.
pub const SAVE_VERSION: i64 = 2;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct State {
    path: PathBuf,
    pub save: Value,
}

impl State {
    /// Loads the save file, or creates a fresh one. `player_default` is the default
    /// `Player` object (from api-defaults.json). Saves older than `SAVE_VERSION` (version 1
    /// had every card unlocked) are replaced by a fresh progression save.
    pub fn load_or_create(path: PathBuf, player_default: Value, prog: &Progression) -> Self {
        if let Some(save) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) {
            if save["version"].as_i64().unwrap_or(1) >= SAVE_VERSION {
                let mut state = State { path, save };
                // Inventory size comes from the config (no expansion purchases offline).
                state.save["player"]["maxCardInvenCount"] = prog.cfg()["start"]["maxCardInventory"].clone();
                // Earlier versions put the starter pack in the save at creation.
                let has_cards = state.save["cards"].as_array().is_some_and(|c| !c.is_empty());
                if has_cards && state.save["claimed"]["starterPack"].is_null() {
                    state.save["claimed"]["starterPack"] = json!(true);
                }
                // Past the tutorial without the pack (e.g. a save created without a progression
                // config, or the step reply got lost): grant it now, without the reveal.
                if !has_cards && state.save["player"]["tutorialStep"].as_i64().unwrap_or(0) >= 1 {
                    prog.starter_provides(&mut state);
                }
                // Live themes earned before this was implemented (or after a config change).
                prog.live_theme_unlocks(&mut state, "liveThemeReward");
                // Saves from before the leader card fix have card 0, which doesn't exist and
                // makes the game hang after the result screen ("card is null. cardId:0").
                if state.save["player"]["leaderCard"]["c"].as_i64().unwrap_or(0) == 0 {
                    if let Some(first) = state.save["cards"].get(0).map(|c| c["objectID"].as_i64().unwrap_or(0)) {
                        state.set_leader(first);
                    }
                }
                return state;
            } else {
                let _ = std::fs::rename(&path, path.with_extension("v1.json")); // keep the old save
            }
        }
        let created = now_ms();
        let start = &prog.cfg()["start"];
        let mut player = match player_default {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        player.insert("objectID".into(), json!(USER_OBJECT_ID));
        player.insert("nickname".into(), json!(start["nickname"].as_str().unwrap_or(NICKNAME)));
        player.insert("isRegularUser".into(), json!(true));
        player.insert("language".into(), json!(1)); // Language.enUS
        player.insert("device".into(), json!(2)); // Channel.ANDROID
        player.insert("maxCardInvenCount".into(), start["maxCardInventory"].clone());
        player.insert("lastLogin".into(), json!(created));
        // Everything is kept in the "free" balances; paid balances stay 0.
        for (paid, free, key) in [("rhythmPoint", "freeRhythmPoint", "rhythmPoint"),
                                  ("diamond", "freeDiamond", "diamond"),
                                  ("headphone", "freeHeadphone", "headphone")] {
            player.insert(paid.into(), json!(0));
            player.insert(free.into(), json!(start[key].as_i64().unwrap_or(0)));
        }
        let state = State {
            path,
            save: json!({
                "version": SAVE_VERSION,
                "createdAt": created,
                "player": player,
                "cards": [],
                "nextCardObjectId": 1000,
                "cardBook": {},
                "records": {},
                "firstClears": {},
                "counters": {"songsPlayed": 0, "songsCleared": []},
                "claimed": {"recordTier": 0, "achievements": [], "starterPack": false},
                "nextMusicPlayId": 1,
            }),
        };
        // No cards yet: the tutorial pack comes with tutorialStepComplete(1) (starter_provides).
        state.persist();
        state
    }

    /// Makes the card with this inventory object ID the player's leader card
    /// (`User.leaderCard` = {c: cardID, g: grade, l: level}). Unknown IDs are ignored.
    pub fn set_leader(&mut self, object_id: i64) {
        let card = self.save["cards"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["objectID"].as_i64() == Some(object_id)))
            .cloned();
        if let Some(c) = card {
            self.save["player"]["leaderCard"] = json!({"c": c["cardID"], "g": c["grade"], "l": c["level"]});
            self.persist();
        }
    }

    pub fn persist(&self) {
        let tmp = self.path.with_extension("tmp");
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, serde_json::to_vec_pretty(&self.save).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// The player as sent at login. Name changes are free offline: the client uses a free
    /// change when nicknameChanger > 0 (otherwise it deducts diamonds itself), and decrements
    /// its own copy after each change, so the server always reports plenty.
    pub fn player(&self) -> Value {
        let mut p = self.save["player"].clone();
        p["nicknameChanger"] = json!(FREE_NICKNAME_CHANGES);
        p
    }

    pub fn cards(&self) -> Value {
        self.save["cards"].clone()
    }

    pub fn next_play_id(&mut self) -> i64 {
        let id = self.save["nextMusicPlayId"].as_i64().unwrap_or(1);
        self.save["nextMusicPlayId"] = json!(id + 1);
        id
    }

    /// Keeps the best score per song and difficulty, plus the best star count and all-perfect.
    /// `judgements` = [miss, good, perfect, superPerfect] as sent with stageComplete.
    pub fn record_score(&mut self, music: i64, difficulty: i64, score: i64, combo: i64, judgements: [i64; 4]) {
        let key = format!("{music}_{difficulty}");
        let rec = &mut self.save["records"][&key];
        let (stars, all_perfect) = (rec["stars"].as_i64().unwrap_or(0), rec["allPerfect"].as_bool().unwrap_or(false));
        if rec["score"].as_i64().unwrap_or(-1) < score {
            *rec = json!({"musicCode": music, "difficult": difficulty, "score": score,
                          "combo": combo, "updatedAt": now_ms()});
        }
        let plays = &mut self.save["playCounts"][music.to_string()];
        *plays = json!(plays.as_i64().unwrap_or(0) + 1);
        let rec = &mut self.save["records"][&key];
        let [miss, good, ..] = judgements;
        rec["stars"] = json!(stars.max(star_count(judgements)));
        rec["allPerfect"] = json!(all_perfect || (miss == 0 && good == 0 && judgements.iter().sum::<i64>() > 0));
        self.persist();
    }
}

/// Stars as the game's result screen counts them (IngameScene.WriteResultData): miss rate =
/// misses / all notes * 100, compared with MusicData's three/two/oneStarMaxMiss, which are
/// 0 / 5 / 100 for every song.
pub fn star_count(judgements: [i64; 4]) -> i64 {
    let total: i64 = judgements.iter().sum();
    let miss_rate = if total > 0 { judgements[0] as f64 / total as f64 * 100.0 } else { 0.0 };
    if miss_rate <= 0.0 { 3 } else if miss_rate <= 5.0 { 2 } else { 1 }
}
