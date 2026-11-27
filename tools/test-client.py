#!/usr/bin/env python3
"""Smoke test for the Rust server: walks the game's startup, login and song-play calls.

Usage: tools/test-client.py --game <id> [http://127.0.0.1:18384]
Exits non-zero on the first failed check.
"""

import base64, gzip, json, random, string, sys, urllib.request
from pathlib import Path
from Crypto.Cipher import AES
from Crypto.Util.Padding import pad, unpad

sys.path.insert(0, str(Path(__file__).resolve().parent))
from game import select
from gamekey import game_key

GAME, ARGV = select()
VERSION = GAME["version"]
PROG = json.load(open(GAME.file("progression.json")))
NICK = PROG["start"].get("nickname", "Player")
BASE = (ARGV[1] if len(ARGV) > 1 else "http://127.0.0.1:18384").rstrip("/")

KEY = game_key()  # from your libssnative.so, see tools/gamekey.py
failures = 0


def check(cond, what):
    global failures
    print(("  ok  " if cond else "  FAIL ") + what)
    if not cond:
        failures += 1


def get(path):
    with urllib.request.urlopen(BASE + path) as r:
        return r.read()


def call(clazz, method, params=(), encrypt=False):
    body = json.dumps({"class": clazz, "method": method, "params": list(params)}).encode()
    iv = "".join(random.choice(string.ascii_letters) for _ in range(16))
    if encrypt:
        body = base64.b64encode(AES.new(KEY, AES.MODE_CBC, iv.encode()).encrypt(pad(body, 16)))
    req = urllib.request.Request(
        BASE + "/",
        data=body,
        headers={
            "X-SuperStar-AES-IV": iv,
            "X-SuperStar-API-Version": "1",
            "X-SuperStar-Asset-Version": "522",
            "X-SuperStar-Asset-Ignore": "false",
        },
    )
    with urllib.request.urlopen(req) as r:
        resp = json.loads(r.read())
    check(resp.get("code") == 1000, f"{clazz}.{method} -> code {resp.get('code')}")
    return resp.get("result")


def decrypt_table(raw):
    if raw[:2] == b"\x1f\x8b":
        raw = gzip.decompress(raw)
    return json.loads(unpad(AES.new(KEY, AES.MODE_ECB).decrypt(base64.b64decode(raw)), 16))


URLS = {}  # code -> URL, for games whose tables refer to the URLs table by code (SSGF)


def local(url):
    if isinstance(url, int):
        url = URLS[url]
    check(url.startswith(BASE + "/cdn/"), f"URL rewritten to local: {url[:90]}")
    return url[len(BASE) :]


print("manifest")
m = json.loads(get(f"/version/real/manifest/{VERSION}.txt?t=1"))
check(m["ServerUrl"] == BASE + "/", f"ServerUrl = {m['ServerUrl']}")
check(m["Cryption"] == "0", "Cryption = 0")

print("startup calls")
check(isinstance(call("Platform", "getTimestamp"), int), "timestamp is an int")
check(call("Platform", "getRegion")["region"] == "KR", "region")
ctx = call("Platform", "checkAssetBundle", [-1])
check(len(ctx["context"]) > 70, f"asset context has {len(ctx['context'])} tables")

print("master data download")
if "URLs" in ctx["context"]:
    URLS.update({u["code"]: u["url"] for u in decrypt_table(get(local(ctx["context"]["URLs"]["file"])))})
music = decrypt_table(get(local(ctx["context"]["MusicData"]["file"])))
check(len(music) > 50, f"MusicData has {len(music)} songs")
song = music[0]
check(get(local(song["sound"]))[:4] == b"OggS", f"song audio ({song['code']}) is OGG")
check(len(get(local(song["seqHard"]))) > 1000, "hard chart downloads")
cards = decrypt_table(get(local(ctx["context"]["CardData"]["file"])))
store = decrypt_table(get(local(ctx["context"]["CardStoreData"]["file"])))
SONGS = [m["code"] for m in music[:6]]
check(get(local(cards[0]["cardImageLarge"]))[:4] == b"\x89PNG", "card image is PNG")
host = local(cards[0]["cardImageLarge"]).split("/")[2]
check(get(f"/cdn/{host}/resources/images/missing_for_test.png")[:4] == b"\x89PNG", "missing image -> placeholder PNG")

print("login")
auth = call("Account", "createAuthentication")
check(auth["userObjectID"] == 1, "createAuthentication")
call("Account", "createAccount", [0, auth["userID"], auth["localPassword"], "whatever", 1, 2, True])
login = call("Account", "login", [0, auth["userID"], auth["localPassword"], VERSION, 2, True], encrypt=True)
check(login["user"]["nickname"] == NICK, f"nickname is {NICK} (encrypted request)")
check(login["user"]["objectID"] == 1, "player objectID 1")

print("post-login steps")
check(len(call("CardInventory", "getCardList", [1, None])) == 0, "starts with no cards (pack comes with the tutorial)")
for clazz, method in [
    ("LiveThemeBook", "getList"),
    ("Game", "getStageStatus"),
    ("CardBook", "getList"),
    ("Store", "getConsumableItemCount"),
    ("LeagueWeekly", "getRank"),
    ("Store", "getPurchaseEvent"),
    ("Store", "getBuyCount"),
    ("Account", "getBuyProfileImage"),
    ("Promotion", "getCollectEventInfo"),
    ("StarPass", "getPassStatus"),
    ("Store", "getStoreCardPackPoint"),
    ("Store", "getMyStoreInfo"),
    ("Store", "getMyArtistPack"),
    ("Account", "getMyInfo"),
    ("Account", "getUserMusicContentsStatus"),
    ("Account", "getBuffStatus"),
    ("Store", "checkWeeklyBuyCount"),
]:
    call(clazz, method, [1])

print("profile and tutorial")
check(len(call("Account", "getBuyProfileImage", [1, None])) > 300, "all profile pictures owned")
call("Account", "changeProfileImage", [1, None, 2006])
check(call("Game", "tutorialStepComplete", [1, None, 0]) == [], "tutorial step 0: no reward")
step1 = call("Game", "tutorialStepComplete", [1, None, 1])
check(
    len(step1) == 1
    and step1[0]["tag"] == "tutorialNormalCardPack30"
    and step1[0]["item"] == 0
    and len(step1[0]["context"]) == 30,
    "tutorial step 1 (profile chosen) returns the normal 30-pack",
)
check(call("Game", "tutorialStepComplete", [1, None, 1]) == [], "... only once")
inv = call("CardInventory", "getCardList", [1, None])
check(len(inv) == 30, f"inventory has the tutorial pack: {len(inv)} cards")
check(sum(1 for c in inv if c["grade"] >= 1) >= 6, "tutorial 30-pack has at least 6 B+ cards")
check(all(c["energy"] > 0 and c["score"] > 0 for c in inv), "cards have stats from CardBalanceData")
user = call("Account", "login", [0, "offline", "offline", 2, VERSION, False])["user"]
check(user["profileImage"] == 2006 and user["tutorialStep"] == 1, "profile picture and tutorial step saved")
check(user["leaderCard"]["c"] != 0, "leader card is a real card")

print("play a song")
start = call("Game", "stageStart", [1, None, SONGS[0], 2, False])
check(start["musicID"] > 0, f"stageStart play id {start['musicID']}")
call(
    "Game",
    "stageComplete",
    [1, None, start["musicID"], 987654, 0, 1, 2, 300, 303, 0, False, 0, 1, 2, 300, "[]", "[]", False],
)
status = call("Game", "getStageStatus", [1])
rec = [r for r in status["musicList"] if r["musicID"] == SONGS[0]]
check(rec and rec[0]["highscore"] >= 987654, f"high score saved: {rec}")
check(rec and rec[0].get("hardScoreGrade") == 8, "no misses on hard: three stars (HARD_THREE_STAR)")

print("progression")


def balance():
    u = call("Account", "login", [0, "offline", "offline", 2, VERSION, False])["user"]
    return u["freeRhythmPoint"] + u["rhythmPoint"], u["freeDiamond"] + u["diamond"]


rp0, dia0 = balance()
rewards = []
for i in range(5):  # songs 2..6 (song 1 above): milestone at song 5
    st = call("Game", "stageStart", [1, None, SONGS[1 + i], 0, 0, False])
    r = call(
        "Game",
        "stageComplete",
        [1, None, st["musicID"], 200000, 0, 1, 2, 300, 303, 0, False, 0, 1, 2, 300, "[]", "[]", False],
    )
    rewards.append(r)
tags = [p["tag"] for r in rewards for p in r["reward"]]
check(tags.count("music_play") == 5, "every song pays RP (music_play)")
first = [
    p for r in rewards for p in r["reward"] if p["tag"] == "clearReward" and p["item"] == 0 and len(p["context"]) == 1
]
check(
    len(first) >= 5 and all(r["firstClearReward"] is None for r in rewards),
    "first clear of each song gives a card (clearReward pack, no firstClearReward)",
)
m5 = next(m for m in PROG["songMilestones"]["list"] if m["every"] == 5)["rhythmPoint"]
mile = [
    p for r in rewards for p in r["reward"] if p["tag"] == "clearReward" and p["item"] == 10 and p["quantity"] == m5
]
check(len(mile) == 1, f"song-count milestone after 5 songs ({m5:,} RP)")
rp1, dia1 = balance()
gained = sum(p["quantity"] for r in rewards for p in r["reward"] if p["item"] == 10)
check(rp1 - rp0 == gained, f"server RP matches the provides: +{gained}")

row = next(r for r in store if r["paymentCategory"] == 0 and r["quantity"] == 5)  # the RP 5-pack
pack = call("Store", "buyCardPack", [1, None, row["code"]])
check(len(pack) == 2 and len(pack[0]["context"]) == 5, "bought a normal 5-pack with RP")
check(pack[1]["item"] == 10 and pack[1]["quantity"] == -row["price"], "cost comes back as a negative RP provide")
check(any(c["grade"] >= 1 for c in pack[0]["context"]), "5-pack guarantee: at least one B")
check(balance()[0] == rp1 - row["price"], "RP deducted on the server")

inv = call("CardInventory", "getCardList", [1, None])
target, mats = inv[0], [c["objectID"] for c in inv[1:4]]
# Same shape as the real game: materials as a JSON string, then probabilityRate, priceRate.
res = call("CardInventory", "intensify", [1, None, target["objectID"], json.dumps(mats), 1.0, 1.0])
check(
    len(res["intensifyResult"]) == 3 and res["card"]["objectID"] == target["objectID"],
    "power-up returns results and the card",
)
check(set(res["intensifyResult"]) <= {0, 1}, "results are CardIntensifyResult codes (0 success, 1 failed)")
check(res["card"]["level"] - target["level"] == res["intensifyResult"].count(0), "each success raised the level")
check(res["provide"]["quantity"] <= 0, f"power-up costs RP ({res['provide']['quantity']})")
inv2 = call("CardInventory", "getCardList", [1, None])
check(not any(c["objectID"] in mats for c in inv2), "power-up materials consumed")

sold = inv2[-1]["objectID"]
before = balance()[0]
call("CardInventory", "sells", [1, None, [sold]])
check(balance()[0] > before, "selling a card gives RP")
book = call("CardBook", "getList", [1, None, 1])
check(len(book["card"]) >= 30, f"card book lists collected cards ({len(book['card'])})")
rw = call("CardBook", "getReward", [1, None, 0, 0])
check(rw and rw[0]["quantity"] > 0, "collection achievement pays out once")
check(call("CardBook", "getReward", [1, None, 0, 0]) == [], "... and only once")

print(f"\n{'all checks passed' if not failures else f'{failures} check(s) failed'}")
sys.exit(1 if failures else 0)
