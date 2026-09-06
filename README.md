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

## 実機で確認した MIDI 割り当て

ポートは `APC mini mk2 Control`（`Notes` ではない）。

| 要素 | 割り当て |
|---|---|
| パッド | note 0 = 左下、右へ +1、上へ +8 |
| 右列（シーン） | note 112（最上）〜 119（最下） |
| 下段（トラック） | note 100（最左）〜 107 |
| Shift | note 122（右下隅。下段 8 個とは別のボタン） |
| フェーダー | CC 48〜55、CC 56 = マスター。0〜127 |

LED は Note On で、**velocity = パレット番号、MIDI チャンネル = 明度と点滅の挙動**。

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
