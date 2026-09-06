# herdr-apc-mini

AKAI APC mini mk2 を [herdr](https://github.com/hashrock/herdr) の物理コンソールにする plugin。

設計は司令塔リポジトリの `pilot/missions/apc-mini-mk2.md` にある。

## 現状

読み取り専用のダッシュボード。**最下段（フェーダーの真上）にプロジェクトの状態ランプ**を描く。

| 状態 | 色 | 挙動 |
|---|---|---|
| `blocked` | 赤 | 点滅 |
| `working` | オレンジ | パルス |
| `done` | 緑 | パルス |
| `idle` | 緑 | 暗め |
| `unknown` / エージェント不在 | 灰 | 暗め |

入力処理はまだ無い。

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
