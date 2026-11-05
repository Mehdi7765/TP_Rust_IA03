# TP Rust embarqué — IA03

TP de Rust embarqué sur carte **STM32 Nucleo-H753ZI** (UTT, IA03) : un feu de circulation piloté par les 3 LEDs de la carte et le bouton utilisateur.

Mehdi Ez-Zouak, Paul Louis Ledoux et Clément Menaucourt

## Contenu

| Dossier | Description |
|---|---|
| `hello-rust/` | Vérification de l'installation de Rust (`cargo new`) |
| `TP_Partie1_FeuCirculation/template_projet_p1/` | Partie 1 : feu tricolore, bouton lu par *polling* |
| `TP_Partie2_Interruptions/template_projet_p2/` | Partie 2 : bouton géré par interruption matérielle (RTIC), code dans `src/light_signal_rtic_template.rs` |
| `IA embarquée  reconnaissance vocale - Partie 1-20261104/` | IA embarquée, partie 1 : lecture du microphone (rien à coder), réponses dans `REPONSES.md` |
| `docs/Explications_TP_Rust.pdf` | Explication simple du code et du TP (à lire en premier) |

## Matériel

- LED verte : PB0, LED jaune : PE1, LED rouge : PB14
- Bouton utilisateur (bleu) : PC13, état haut quand il est appuyé
- Brancher la carte par le port USB ST-LINK (débogage)

## Installation

```sh
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# Cible ARM Cortex-M7
rustup target add thumbv7em-none-eabihf
# probe-rs (flash + console RTT)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh
```

Sous Linux, il faut aussi la règle udev de probe-rs pour accéder à la sonde sans `sudo` :

```sh
curl -fsSL https://probe.rs/files/69-probe-rs.rules -o 69-probe-rs.rules
sudo cp 69-probe-rs.rules /etc/udev/rules.d/ && sudo udevadm control --reload && sudo udevadm trigger
```

## Lancer un TP

```sh
cd TP_Partie1_FeuCirculation/template_projet_p1   # ou la partie 2
cargo run
```

`cargo run` compile, flashe la carte et affiche la console RTT (Ctrl+C pour quitter). Ne pas utiliser `--release`.

## Partie 1 — Feu de circulation (polling)

- Cycle normal accéléré : vert 3 s → jaune 1 s → rouge 3 s.
- Bouton maintenu : dérangement, le jaune clignote (0,5 s). Au relâchement, le cycle reprend au rouge.
- La boucle avance par pas de 10 ms et lit le bouton à chaque pas : la réaction est quasi immédiate.

**Piège rencontré :** le `Delay` de `stm32h7xx-hal` 0.16 suppose une puce en révision Y. Nos cartes sont en **révision V** (`DBGMCU_IDC.REV_ID = 0x2003`), dont le SysTick est divisé par 8 : tous les délais duraient 8 fois trop longtemps (le template fait clignoter toutes les 4 s au lieu de 0,5 s). L'option `revision_v` de la HAL ne compile pas, on utilise donc `cortex_m::delay::Delay`, cadencé par l'horloge du processeur.

## Partie 2 — Interruptions avec RTIC

- Le bouton PC13 est configuré en source d'interruption EXTI (ligne 13), sur les **deux fronts** (`Edge::RisingFalling`) pour détecter l'appui et le relâchement.
- La tâche `btn_isr` est liée au vecteur `EXTI15_10` (lignes EXTI 10 à 15). Elle lit le niveau du bouton, met à jour la ressource partagée `light_failed` (via `lock`) et acquitte l'interruption (`clear_interrupt_pending_bit`).
- `idle` pilote les LEDs à partir de `light_failed`. L'interruption est immédiate, mais les LEDs ne changent qu'à la fin du `delay_ms` en cours dans `idle` (comportement attendu).
- Les délais passent par le timer TIM2, qui n'a pas le problème de révision de la partie 1.
