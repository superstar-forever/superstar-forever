//! The game's RPC endpoint: `POST <ServerUrl>` with `{"class", "method", "params"}`.
//!
//! Calls with a handler below get real answers; every other known call gets the default
//! result from api-defaults.json (empty lists, zeros), and unknown calls get `null`.
//! Every call is logged so missing or wrong answers can be found when testing the app.

use crate::assets::Assets;
use crate::crypto;
use crate::progression::{self as pg, Progression};
use crate::state::{now_ms, State, NICKNAME, USER_OBJECT_ID};
use serde_json::{json, Map, Value};
use std::sync::Mutex;

pub const OK: i64 = 1000;

pub struct Api {
    defaults: Map<String, Value>,
    state: Mutex<State>,
    /// (musicCode, difficulty) of the stage in progress; stageComplete doesn't repeat them.
    current_stage: Mutex<Option<(i64, i64)>>,
    /// Every profile picture code (ProfileData), all treated as bought.
    profile_codes: Vec<i64>,
    prog: Progression,
}

pub struct Reply {
    pub body: Vec<u8>,
    pub log: String,
}

impl Api {
    pub fn new(defaults: Map<String, Value>, state: State, profile_codes: Vec<i64>, prog: Progression) -> Self {
        Api { defaults, state: Mutex::new(state), current_stage: Mutex::new(None), profile_codes, prog }
    }

    /// The save file contents without the card list (for GET /save).
    /// The whole save, for GET /save/full (backups and editing with tools/save.py).
    pub fn export_save(&self) -> Value {
        self.state.lock().map(|s| s.save.clone()).unwrap_or(Value::Null)
    }

    /// Replaces the save (PUT /save/full). The game shows the change after a restart.
    pub fn import_save(&self, body: &[u8]) -> Result<(), String> {
        let save: Value = serde_json::from_slice(body).map_err(|e| format!("not JSON: {e}"))?;
        if save["version"].as_i64().unwrap_or(0) < crate::state::SAVE_VERSION
            || !save["player"].is_object()
            || !save["cards"].is_array()
        {
            return Err(format!("not a save (needs version >= {}, player, cards)", crate::state::SAVE_VERSION));
        }
        let mut s = self.state.lock().map_err(|_| "state unavailable".to_string())?;
        s.save = save;
        s.persist();
        Ok(())
    }

    pub fn save_summary(&self) -> Value {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return Value::Null,
        };
        let mut save = state.save.clone();
        if let Some(obj) = save.as_object_mut() {
            let n = obj.get("cards").and_then(|c| c.as_array()).map_or(0, |c| c.len());
            obj.insert("cards".into(), json!(format!("{n} cards (omitted)")));
        }
        save
    }

    /// Handles one request body. `iv` is the X-SuperStar-AES-IV header, if any.
    pub fn handle(&self, assets: &Assets, body: &[u8], iv: Option<&str>) -> Reply {
        let plain = if body.first() == Some(&b'{') {
            body.to_vec()
        } else {
            match iv.map(|iv| crypto::decrypt_api(body, iv)) {
                Some(Ok(p)) => p,
                Some(Err(e)) => return error_reply(format!("decrypt failed: {e}")),
                None => return error_reply("encrypted body without X-SuperStar-AES-IV".into()),
            }
        };
        let req: Value = match serde_json::from_slice(&plain) {
            Ok(v) => v,
            Err(e) => return error_reply(format!("bad JSON: {e}")),
        };
        let class = req["class"].as_str().unwrap_or("");
        let method = req["method"].as_str().unwrap_or("");
        let params = req["params"].as_array().cloned().unwrap_or_default();
        let key = format!("{class}.{method}");

        let (result, source) = match self.dispatch(assets, &key, &params) {
            Some(r) => (r, "handler"),
            None => match self.defaults.get(&key) {
                Some(d) => (d.clone(), "default"),
                None => (Value::Null, "UNKNOWN"),
            },
        };
        let body = json!({"code": OK, "result": result, "invoke": []}).to_string().into_bytes();
        let mut p = Value::Array(params).to_string();
        truncate(&mut p, 300);
        Reply { log: format!("{key} [{source}] params={p}"), body }
    }

    fn dispatch(&self, assets: &Assets, key: &str, p: &[Value]) -> Option<Value> {
        // Calls made after login send [userObjectID, accessToken, ...wrapper arguments];
        // `arg(n)` is the n-th wrapper argument (0-based).
        let arg = |n: usize| p.get(n + 2).and_then(|v| v.as_i64()).unwrap_or(0);
        let argf = |n: usize| p.get(n + 2).and_then(|v| v.as_f64()).unwrap_or(1.0);
        // long[] arguments arrive as JSON arrays; some calls send them as a JSON string.
        let ids = |n: usize| -> Vec<i64> {
            let v = p.get(n + 2).cloned().unwrap_or(Value::Null);
            let v = match v {
                Value::String(t) => serde_json::from_str(&t).unwrap_or(Value::Null),
                other => other,
            };
            v.as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default()
        };
        let set_player = |field: &str, value: Value| -> Option<Value> {
            let mut s = self.state.lock().ok()?;
            s.save["player"][field] = value;
            s.persist();
            Some(Value::Null)
        };
        Some(match key {
            // ---- startup
            "Platform.getTimestamp" => json!(now_ms()),
            "Platform.getRegion" => json!({"region": "KR", "isStreaming": false}),
            "Platform.getClientIP" => json!("127.0.0.1"),
            "Platform.checkAssetBundle" => assets.asset_context(),

            // ---- account: one fixed player, whatever the client sends
            "Account.createAuthentication" => json!({
                "userObjectID": USER_OBJECT_ID, "provider": 0,
                "userID": "offline", "localPassword": "offline"}),
            "Account.createAccount" | "Account.createSNSAccount" => Value::Null,
            "Account.login" | "Account.loginSNS" => self.login_response(),
            "Account.getUserData" => self.state.lock().ok()?.player(),
            "Account.isValidNickname" => Value::Null,
            "Account.changeNickname" => {
                // The client checks length and characters itself (GameConfig NICKNAME_*_LENGTH).
                let name = p.get(2).and_then(Value::as_str).unwrap_or("").trim().to_string();
                if name.is_empty() {
                    return None;
                }
                set_player("nickname", json!(name))?
            }
            "Account.changeMyMember" => set_player("myMember", json!(arg(0)))?,
            "Account.changeLanguage" => set_player("language", json!(arg(0)))?,
            "LeagueWeekly.getRank" => self.weekly_rank()?,
            "Promotion.getNewsData" => self.news(assets)?,
            "Account.refreshAccessToken" | "Account.applyAccessToken" => Value::Null,

            // ---- profile and lobby: everything owned, choices remembered
            "Account.getBuyProfileImage" => json!(self.profile_codes),
            "Account.changeProfileImage" => set_player("profileImage", json!(arg(0)))?,
            "Account.changeMyLobbyBg" => set_player("myLobbyBg", json!(arg(0)))?,
            // Wallpapers and artist packs are cosmetic: everything is owned offline.
            "Account.getMyLobbyBgList" => json!(self.prog.lobby_bgs.iter().map(|c| json!({"bgCode": c})).collect::<Vec<_>>()),
            "Account.changeLobbyBgList" => {
                // The lobby rotates through the equipped wallpapers (Player.equipedLobbyBgList).
                let mut s = self.state.lock().ok()?;
                let mut list: Vec<i64> = s.save["player"]["equipedLobbyBgList"]
                    .as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                let (code, equip) = (arg(0), p.get(3).and_then(Value::as_bool).unwrap_or(true));
                list.retain(|&c| c != code);
                if equip {
                    list.push(code);
                }
                s.save["player"]["equipedLobbyBgList"] = json!(list);
                s.persist();
                Value::Null
            }
            "Store.getMyArtistPack" => {
                let created = self.state.lock().ok()?.save["createdAt"].clone();
                json!(self.prog.artist_packs.iter()
                    .map(|c| json!({"owner": 1, "artistPackCode": c, "createdAt": created}))
                    .collect::<Vec<_>>())
            }
            "Store.equipArtistPack" => {
                let equip = p.get(3).and_then(Value::as_bool).unwrap_or(true);
                set_player("artistPack", if equip { json!(arg(0)) } else { Value::Null })?
            }
            "Game.tutorialStepComplete" => {
                let mut s = self.state.lock().ok()?;
                let cur = s.save["player"]["tutorialStep"].as_i64().unwrap_or(-1);
                s.save["player"]["tutorialStep"] = json!(cur.max(arg(0)));
                s.persist();
                // Step 1 = profile picture chosen (after the tutorial song, or after skipping it):
                // the client opens the returned rewards, so the tutorial pack goes here.
                if arg(0) >= 1 { json!(self.prog.starter_provides(&mut s)) } else { json!([]) }
            }

            // ---- cards (see progression.rs)
            "CardInventory.getCardList" => self.state.lock().ok()?.cards(),
            "Store.buyCardPack" => {
                let mut st = self.state.lock().ok()?;
                self.prog.buy_card_pack(&mut st, arg(0)).unwrap_or_else(|_| json!([]))
            }
            "Store.buyRhythmPoint" => {
                let mut st = self.state.lock().ok()?;
                self.prog.buy_rhythm_point(&mut st, arg(0)).unwrap_or_else(|_| json!([]))
            }
            "CardInventory.intensify" => {
                // Wire order (the wrapper's object[], not its signature): intensifyCardObjectID,
                // materialCardObjectID[] (as a JSON string), probabilityRate, priceRate.
                let mut st = self.state.lock().ok()?;
                self.prog.intensify(&mut st, arg(0), &ids(1), argf(2), argf(3))?
            }
            "CardInventory.upgrade" => {
                // Wire order: requestCardObjectID, targetCardObjectID, priceRate.
                let mut st = self.state.lock().ok()?;
                self.prog.grade_up(&mut st, arg(0), arg(1), argf(2)).ok()?
            }
            "CardInventory.sell" => {
                let mut st = self.state.lock().ok()?;
                self.prog.sell(&mut st, &[arg(0)]);
                Value::Null
            }
            "CardInventory.sells" => {
                let mut st = self.state.lock().ok()?;
                self.prog.sell(&mut st, &ids(0));
                Value::Null
            }
            "CardInventory.lock" | "CardInventory.multiLock" => {
                let list = if key.ends_with("multiLock") { ids(0) } else { vec![arg(0)] };
                let on = p.get(3).and_then(Value::as_bool).unwrap_or(true);
                pg::set_card_flag(&mut *self.state.lock().ok()?, &list, "isLock", on);
                Value::Null
            }
            "CardInventory.equip" | "CardInventory.multiEquip" | "CardInventory.unequip" | "CardInventory.multiUnequip" => {
                let list = if key.contains("multi") { ids(0) } else { vec![arg(0)] };
                pg::set_card_flag(&mut *self.state.lock().ok()?, &list, "isEquip", !key.contains("nequip"));
                Value::Null
            }
            "CardBook.getList" => self.prog.card_book(&*self.state.lock().ok()?),
            "CardBook.regist" => {
                // Cards are registered when created; this call only lists what the client has.
                let mut st = self.state.lock().ok()?;
                for id in ids(0) {
                    if let Some(c) = pg::find_card(&st, id) {
                        pg::register_card_book(&mut st, c["cardID"].as_i64().unwrap_or(0), c["grade"].as_i64().unwrap_or(0));
                    }
                }
                st.persist();
                json!([])
            }
            "CardBook.getReward" => {
                let mut st = self.state.lock().ok()?;
                self.prog.card_book_reward(&mut st, arg(0), arg(1))
            }

            // ---- live themes (earned by completing albums, Progression::live_theme_unlocks)
            "LiveThemeBook.getList" => self.prog.live_theme_list(&*self.state.lock().ok()?),
            "LiveThemeBook.equipCard" | "LiveThemeBook.equipBackground" => {
                // Wire order: objectID, on. A background is one card at a time.
                let field = if key.ends_with("Background") { "isBackground" } else { "isEquip" };
                let on = p.get(3).and_then(Value::as_bool).unwrap_or(true);
                let mut st = self.state.lock().ok()?;
                for c in st.save["liveThemes"]["cards"].as_array_mut()?.iter_mut() {
                    if c["objectID"].as_i64() == Some(arg(0)) {
                        c[field] = json!(on);
                    } else if field == "isBackground" && on {
                        c[field] = json!(false);
                    }
                }
                st.persist();
                Value::Null
            }
            "LiveThemeBook.setLeader" => {
                let on = p.get(3).and_then(Value::as_bool).unwrap_or(true);
                let mut st = self.state.lock().ok()?;
                st.save["liveThemes"]["leader"] = if on { json!(arg(0)) } else { Value::Null };
                st.persist();
                Value::Null
            }

            // ---- playing
            "Game.getStageStatus" => self.stage_status(),
            "Game.getFavorites" => self.favorites()?,
            "Game.setFavorites" => {
                // Wire order (the wrapper's object[], not its signature): musicAdd, groupAdd,
                // musicRemove, groupRemove (lists of codes).
                let mut st = self.state.lock().ok()?;
                for (key, add, remove) in [("music", ids(0), ids(2)), ("groups", ids(1), ids(3))] {
                    let mut list: Vec<i64> = st.save["favorites"][key].as_array()
                        .map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                    list.retain(|c| !remove.contains(c));
                    for c in add {
                        if !list.contains(&c) {
                            list.push(c);
                        }
                    }
                    st.save["favorites"][key] = json!(list);
                }
                st.persist();
                Value::Null
            }
            "Game.stageStart" | "Game.rehearsalStageStart" => {
                // args: musicCode, difficult, ...
                *self.current_stage.lock().ok()? = Some((arg(0), arg(1)));
                let play_id = self.state.lock().ok()?.next_play_id();
                let mut r = self.defaults.get("Game.stageStart").cloned().unwrap_or(json!({}));
                r["musicID"] = json!(play_id);
                r["startedAt"] = json!(now_ms());
                r["yearMonth"] = json!(year_month());
                r
            }
            "Game.stageComplete" | "Game.rehearsalStageComplete" => {
                // args (seen from the real game): playID (stageStart's musicID), score, miss,
                // good, perfect, sPerfect, combo, leaderCardObjectID, missionVerify, ...
                let (mut rewards, mut first_clear) = (Vec::new(), Value::Null);
                if let Some((music, diff)) = *self.current_stage.lock().ok()? {
                    let mut s = self.state.lock().ok()?;
                    s.record_score(music, diff, arg(1), arg(6), [arg(2), arg(3), arg(4), arg(5)]);
                    if arg(7) != 0 {
                        s.set_leader(arg(7)); // leaderCardObjectID of the deck just played
                    }
                    (rewards, first_clear) = self.prog.stage_rewards(&mut s, music, diff, arg(1));
                }
                let mut r = self.defaults.get("StandardSelect.leagueGiftHeadPhone").cloned().unwrap_or(json!({}));
                // The result screen applies the provides by tag ("music_play" = RP,
                // "clearReward" = first clear, milestones, record ladder). firstClearReward
                // must be a real card or null (card 0 hangs the game); we always send null
                // (see Progression::stage_rewards).
                r["firstClearReward"] = first_clear;
                r["gotchaCardObjectID"] = json!(0);
                r["reward"] = Value::Array(rewards);
                r
            }
            _ => return None,
        })
    }

    /// The weekly league reply from the defaults (games/<game>/api-overrides.json), with the
    /// player's own entries (objectID = USER_OBJECT_ID) showing their current name and leader
    /// card. SSGF finds "my" entry by object ID and nickname; without it the lobby stays empty.
    fn weekly_rank(&self) -> Option<Value> {
        let mut r = self.defaults.get("LeagueWeekly.getRank")?.clone();
        let state = self.state.lock().ok()?;
        let player = &state.save["player"];
        if let Some(list) = r["ranking"].as_array_mut() {
            for e in list.iter_mut().filter(|e| e["objectID"].as_i64() == Some(USER_OBJECT_ID)) {
                e["nickname"] = player["nickname"].clone();
                e["leaderCard"] = player["leaderCard"].clone();
                e["profileImage"] = player["profileImage"].clone();
            }
        }
        Some(r)
    }

    /// Lobby news banners from the defaults (games/<game>/api-overrides.json), with image URLs
    /// pointing to our server (tools/build-server-data.py puts the banner under cdn/offline.local/).
    fn news(&self, assets: &Assets) -> Option<Value> {
        let list = self.defaults.get("Promotion.getNewsData")?;
        serde_json::from_str(&assets.rewrite_urls(&list.to_string())).ok()
    }

    /// Game.getFavorites: hearted songs (linkeds) and groups from the save, play counts per song.
    /// The original filled the *All lists with everyone's likes and plays; offline they're the
    /// player's own (a liked song counts 1).
    fn favorites(&self) -> Option<Value> {
        let st = self.state.lock().ok()?;
        let mut r = self.defaults.get("Game.getFavorites").cloned().unwrap_or(json!({}));
        let list = |k: &str| st.save["favorites"][k].as_array().cloned().unwrap_or_default();
        let plays: Vec<Value> = st.save["playCounts"].as_object().map(|m| m.iter()
            .filter_map(|(k, v)| Some(json!({"musicID": k.parse::<i64>().ok()?, "count": v.as_i64().unwrap_or(0)})))
            .collect()).unwrap_or_default();
        let liked: Vec<Value> = list("music").iter().map(|c| json!({"musicID": c, "count": 1})).collect();
        r["linkeds"] = json!(list("music"));
        r["groups"] = json!(list("groups"));
        r["plays"] = json!(plays.clone());
        r["prevPlays"] = json!(plays.clone());
        r["prevPlayAlls"] = json!(plays);
        r["prevLinkedAlls"] = json!(liked);
        Some(r)
    }

    fn login_response(&self) -> Value {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return Value::Null,
        };
        let mut r = self.defaults.get("Account.login").cloned().unwrap_or(json!({}));
        r["nickname"] = match state.save["player"]["nickname"].as_str() {
            Some(n) => json!(n),
            None => json!(NICKNAME),
        };
        r["authObjectID"] = json!(USER_OBJECT_ID);
        r["createdAt"] = state.save["createdAt"].clone();
        r["isRegularUser"] = json!(true);
        r["obtainableMyRecordReward"] = json!(false); // the record ladder is paid out in song results
        r["user"] = state.player();
        r
    }

    fn stage_status(&self) -> Value {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return Value::Null,
        };
        let mut best: Map<String, Value> = Map::new();
        if let Some(records) = state.save["records"].as_object() {
            for rec in records.values() {
                let music = rec["musicCode"].to_string();
                let score = rec["score"].as_i64().unwrap_or(0);
                let entry = best.entry(music).or_insert_with(|| {
                    json!({"musicID": rec["musicCode"], "highscore": 0, "updatedAt": rec["updatedAt"],
                           "seasonCode": 0, "isForceUnlock": false})
                });
                if entry["highscore"].as_i64().unwrap_or(0) < score {
                    entry["highscore"] = json!(score);
                }
                // Stars per difficulty: ScoreGrade = difficulty * 3 + stars - 1 (EASY_ONE_STAR = 0).
                let diff = rec["difficult"].as_i64().unwrap_or(0).clamp(0, 2) as usize;
                let name = ["easy", "normal", "hard"][diff];
                if let Some(stars) = rec["stars"].as_i64().filter(|&n| n > 0) {
                    entry[format!("{name}ScoreGrade")] = json!(diff as i64 * 3 + stars - 1);
                }
                if rec["allPerfect"].as_bool() == Some(true) {
                    entry[format!("{name}AllPerfect")] = json!(true);
                }
            }
        }
        let mut r = self.defaults.get("Game.getStageStatus").cloned().unwrap_or(json!({}));
        r["musicList"] = Value::Array(best.into_values().collect());
        r["yearMonth"] = json!(year_month());
        r
    }
}

fn year_month() -> i64 {
    // Days since epoch -> civil date (Howard Hinnant's algorithm), UTC.
    let days = now_ms().div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    year * 100 + month
}

fn truncate(s: &mut String, max: usize) {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
}

fn error_reply(msg: String) -> Reply {
    Reply {
        body: json!({"code": -1000, "result": null, "invoke": []}).to_string().into_bytes(),
        log: format!("ERROR {msg}"),
    }
}
