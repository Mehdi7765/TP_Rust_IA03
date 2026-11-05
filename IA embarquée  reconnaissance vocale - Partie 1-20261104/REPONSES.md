# IA embarquée : reconnaissance vocale — Partie 1 (Microphone)

Rien à coder dans cette partie : on lance le programme fourni et on observe.

```sh
cd temp_micro_partie1
cargo run --release   # compile, flashe, affiche le vumètre (Ctrl+C pour quitter)
```

## 1. Fréquence d'échantillonnage

**8 010 échantillons/s** (valeur affichée par la carte).

## 2. Durée de 2 000 échantillons

2 000 / 8 010 ≈ **0,25 s (250 ms)**.

## 3. Niveaux sonores et LEDs

Mesures relevées (≈ 36 s, 366 lignes) :

| Situation | Niveau mesuré |
|---|---|
| Silence (bruit du micro) | ≈ 19 000 (< 20 000) |
| Bruit ambiant de la salle (médiane) | ≈ 80 000 |
| Pic le plus fort relevé | ≈ 1 700 000 |
| Voix normale à forte (énoncé) | 50 000 à 1 000 000 |
| Claquement de mains (énoncé) | ≈ 7 000 000 (limite 24 bits : 8 388 607) |

Les LEDs sont testées à **chaque échantillon** :

- **jaune** : échantillon > 2^18 = **262 144**
- **rouge** : échantillon > 2^20 = **1 048 576**

## 4. Pourquoi pas la LED verte ?

Elle est sur **PB0**, qui sert ici d'**horloge envoyée au microphone** (DFSDM1_CKOUT, ≈ 2 MHz). La broche ne peut pas faire les deux. La LED verte peut paraître faiblement allumée : c'est l'horloge.

## 5. Lecture de `src/utils.rs`

- **Broches** (fonction alternative 6) :
  - **PB0 = DFSDM1_CKOUT** : horloge vers le micro
  - **PB10 = DFSDM1_DATIN7** : flux de bits (PDM) renvoyé par le micro
  - Les deux fils blancs relient ces broches à D3/D4 du shield.
- **Filtre** : **sinc3** (`ford().bits(3)`).
- **Suréchantillonnage** : **250** (`fosr().bits(249)`, le registre contient la valeur − 1).
- Vérification : 8 010 × 250 ≈ **2 MHz**, la fréquence de l'horloge du micro.
