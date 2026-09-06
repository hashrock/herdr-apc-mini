# herdr-apc-mini

AKAI APC mini mk2 を [herdr](https://github.com/hashrock/herdr) の物理コンソールにする plugin。

設計は司令塔リポジトリの `pilot/missions/apc-mini-mk2.md` にある。

## 現状

**最下段（フェーダーの真上）にプロジェクトの状態ランプ**を描き、その行だけ入力を受け付ける。

| 状態 | 色 | 挙動 |
|---|---|---|
| `blocked` | 赤 | 点滅 |
| `working` | オレンジ | パルス |
| `done` | 緑 | パルス |
| `idle` | 緑 | 暗め |
| `unknown` / エージェント不在 | 白 | 50% |

### 操作

| 操作 | 動作 |
|---|---|
| パッド単押し | `workspace.focus` — そのワークスペースへ飛ぶ |
| **右列を押しながらパッド** | その動詞をそのワークスペースで実行 |
| Volume (下段 1) | フォーカス中の agent へ **Enter**（`agent.send_keys`） |
| Pan (下段 2) | フォーカス中の agent へ「推奨案で進めて」（`agent.prompt`） |

発火時は白フラッシュ、撃てないときは赤フラッシュ。長押しは使わない。

### 右列 soft key = 「余地」のランプ

動詞のメニューではなく、**その操作に実行の余地が艦隊のどこかにあるか**を示すランプ。

- 緑点灯 = 余地のあるワークスペースが 1 つ以上ある
- 押している間、8×8 が「どこに余地があるか」の地図に変わる（緑 = 余地あり）
- 押しながらパッドで実行

| soft key | 動詞 | 理由がある条件 | 投入するもの |
|---|---|---|---|
| 1 | simplify | 記録した sha ≠ 現在の HEAD | `/simplify` |
| 2 | sync remote | 上流と乖離（ahead / behind） | `リモートと同期して` |

「余地あり」は**理由がある**ことと**今すぐ実行できる**こと（agent がいて `idle`/`done`）の両方。
理由はあるが `working` / `blocked` / agent 不在なら暗いまま。

simplify の記録は config-dir の `simplify.json`。**投入時ではなく、相手が手を止めた
時点の HEAD** を記録する（「済み」は終わった時点のコードに対して成り立つため）。

git の状態は herdr のイベントに現れないので、30 秒ごとにポーリングする。

### 枠の割り当て

割り当ては `~/.config/herdr/plugins/config/apc-mini/pad-map.json` に永続化される。
8x8 の 64 枠に左上から詰める。初回だけエージェントのいるワークスペースを前に置き、以後は順序が動かない。
ワークスペースが消えても穴は空いたままにする。

## MIDI 割り当て

AKAI の [APC mini mk2 Communication Protocol v1.0](https://cdn.inmusicbrands.com/akai/attachments/APC%20mini%20mk2%20-%20Communication%20Protocol%20-%20v1.0.pdf)
と実機の両方で確認済み。ポートは `APC mini mk2 Control`（`Notes` ではない）。

| 要素 | 割り当て |
|---|---|
| パッド | note 0x00-0x3F。**note 0 = 左下**、右へ +1、上へ +8 |
| 下段（トラック） | note 0x64-0x6B (100-107)、最左が 1 |
| 右列（シーン） | note 0x70-0x77 (112-119)、最上が 1 |
| Shift | note 0x7A (122)。**LED は無い** |
| フェーダー | CC 0x30-0x37 (48-55)、CC 0x38 (56) = マスター |

パッドの上下の向きだけは仕様書が図なので、実機で確かめた。

### RGB パッドの LED

Note On で **velocity = パレット番号（128 色固定）、MIDI チャンネル = 挙動**。

| チャンネル | 挙動 |
|---|---|
| 0-6 | 明度 10 / 25 / 50 / 65 / 75 / 90 / 100% |
| 7-10 | Pulsing 1/16, 1/8, 1/4, 1/2 |
| 11-15 | Blinking 1/24, 1/16, 1/8, 1/4, 1/2 |

実機で見ると **ch 0（10%）と ch 1（25%）は暗すぎて見えない**。ch 2（50%）が実用上の下限。

色は 3=白 5=赤 9=橙 13=黄 21=緑 45=青。仕様上 1 は #1E1E1E、2 は #7F7F7F の灰だが、
実機では 1・2・3 のいずれも白っぽく見えるので、くすんだ表現は明度で作る。

### 下段・右列のボタンの LED

**単色 LED で、メッセージ形式が違う。** チャンネルは常に 0（`0x90` 固定）、
velocity は `0x00`=消灯 / `0x01`=点灯 / `0x02`=点滅 の 3 値のみ。
色は選べず、**下段=赤、右列=緑で固定**。RGB パッドと同じ書き方をしてはいけない。

### まだ使っていない仕様

- **Introduction メッセージ**（SysEx `F0 47 7F 4F 60 00 04 00 ...`）を送ると、
  デバイスが **9 本のフェーダーの現在位置を返す**。フェーダーのソフトピックアップを
  初期化するのに使える
- SysEx `F0 47 7F 4F 24 ...` でパレット外の任意 24bit 色を指定できる
- LED の一括更新用メッセージ形式がある

## 開発

```bash
cargo build --release
./target/release/apcd            # 直接動かす

herdr plugin link "$PWD"         # herdr に繋ぐ
herdr plugin enable apc-mini
herdr plugin log list --plugin apc-mini
```

`apcd` は `$HERDR_SOCKET_PATH` に接続し、`pane.updated` を購読する。
この購読は**直後に全ペインの現在状態をリプレイする**ので、初期化のための snapshot 取得は要らない。

MIDI ポートの下見には `cargo run --bin ports`、生の入力を見るには `cargo run --bin probe -- 60`。
