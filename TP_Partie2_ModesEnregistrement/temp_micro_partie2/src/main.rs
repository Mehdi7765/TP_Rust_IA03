// ============================================================================
// DOCUMENTATION UTILE
//
//  API de la HAL (GPIO, EXTI...) :    https://docs.rs/stm32h7xx-hal
//  Livre RTIC (#[idle], #[task], lock) : https://rtic.rs/2/book/en/
//  Livre Rust :                       https://doc.rust-lang.org/book/
//  Votre projet "feu de circulation" (partie RTIC) : même structure !
//
//  Toujours lancer avec : cargo run --release
// ============================================================================
//
// PARTIE 2 : MODES ET ENREGISTREMENT D'UN MOT
//
// Objectif (pas encore de réseau de neurones) :
//   - le bouton bleu bascule entre deux modes : "inference" et "entrainement"
//     (message "MODE : ..." à chaque changement, LED jaune allumée en entraînement)
//   - dès qu'un son est assez fort, enregistrer un mot de 2000 échantillons
//     (250 ms) et afficher "Mot entendu (250 ms enregistres)"
//   - attendre la fin du mot (silence) avant de réécouter
//
// Fourni : src/utils.rs (carte + microphone) et src/microphone.rs (lecture,
// amplitude, historique de 50 ms). Lisez-les avant de commencer !
// ============================================================================

#![no_main]
#![no_std]

mod microphone;
mod utils;

use cortex_m::peripheral::DWT;
use hal::{
    delay::DelayFromCountDownTimer,
    gpio::{ExtiPin, Input, Output, PB14, PC13, PE1, PushPull},
    pac,
    prelude::*,
    timer::Timer,
};
use stm32h7xx_hal as hal;

use defmt::println;
use defmt_rtt as _;
use panic_probe as _;

use microphone::{Microphone, PRE_TRIGGER};

/// Horloge du processeur (laissée par défaut par utils::init : HSI 64 MHz)
const CPU_HZ: u32 = 64_000_000;
/// Échantillons par enregistrement : 2000 à ~8 000 éch/s = 250 ms
const INPUT_SIZE: usize = 2000;
/// Amplitude (sans le décalage) qui déclenche un enregistrement.
/// Silence : < 20 000 ; voix : 50 000 à 1 000 000 ; claquement de mains : jusqu'à 7 000 000
const TRIGGER: u32 = 1 << 17;
/// Échantillons sous TRIGGER nécessaires avant de réécouter (~0,3 s)
const QUIET_SAMPLES: u32 = 2400;
/// Appuis sur le bouton plus rapprochés que ça ignorés (rebonds) : 200 ms
const DEBOUNCE_CYCLES: u32 = CPU_HZ / 5;

#[rtic::app(device = hal::pac, peripherals = true)]
mod app {
    use super::*;

    // TODO 1 : ressource partagée entre `idle` et `btn_isr`.
    //
    // Il faut un booléen `training_mode` (faux au démarrage = mode inférence),
    // modifié par l'interruption du bouton et lu par `idle`.
    // N'oubliez pas de l'initialiser à la fin de `init`.
    #[shared]
    struct Shared {
        // Étape 2 : ressource partagée training_mode (false = inférence, true = entraînement)
        training_mode: bool,
    }

    #[local]
    struct Local {
        delay: DelayFromCountDownTimer<Timer<pac::TIM15>>,
        led_red: PB14<Output<PushPull>>,
        led_yellow: PE1<Output<PushPull>>,
        btn: PC13<Input>,
        dfsdm: pac::DFSDM,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
    // BEGIN CONFIG : NE PAS MODIFIER NI SUPPRIMER
        let dp = cx.device;
        let mut cp = cx.core;

        // Compteur de cycles DWT : compte les cycles d'horloge du processeur
        // (CPU_HZ par seconde). Lecture : `DWT::cycle_count()` (u32, repasse à 0
        // toutes les ~67 s : utiliser `wrapping_sub` pour calculer une durée).
        cp.DCB.enable_trace();
        cp.DWT.enable_cycle_counter();

        // Carte, LED, bouton bleu (déjà configuré en interruption EXTI15_10,
        // front montant) et microphone
        let (delay, led_red, led_yellow, btn, dfsdm) = utils::init(dp);
    // END CONFIG

        (
            // Étape 2 : initialisation de training_mode à false (mode inférence au démarrage)
            Shared {
                training_mode: false,
            },
            Local {
                delay,
                led_red,
                led_yellow,
                btn,
                dfsdm,
            },
        )
    }

    // TODO 2 : la boucle principale `idle`.
    //
    // Au début (avant la boucle) :
    //   - créer le lecteur de microphone : `Microphone::new()` (voir src/microphone.rs)
    //   - créer le tampon d'enregistrement : un tableau de INPUT_SIZE `i32`
    //   - lire ~8000 échantillons pour laisser le filtre du microphone se stabiliser
    //     (~1 s ; allumer les deux LED pendant ce temps est une bonne idée)
    //   - afficher "MODE : inference (bouton bleu pour entrainer)" puis "En ecoute..."
    //
    // Dans la boucle :
    //   a) lire `training_mode` (avec un lock). S'il a changé depuis le dernier tour,
    //      afficher "MODE : entrainement" ou "MODE : inference", allumer/éteindre
    //      la LED jaune, puis "En ecoute..."
    //      (il faut donc garder le mode précédent dans une variable)
    //   b) lire un échantillon : `mic.read(...)` renvoie (valeur, amplitude).
    //      Si l'amplitude est inférieure à TRIGGER, recommencer la boucle (`continue`)
    //   c) sinon, enregistrer un mot dans le tampon :
    //        - les PRE_TRIGGER premières cases : les échantillons d'avant le déclenchement,
    //          avec `mic.copy_history(...)` (une tranche : `&mut buffer[..PRE_TRIGGER]`)
    //        - les cases suivantes : de nouveaux échantillons lus avec `mic.read(...)`
    //      puis afficher "Mot entendu (250 ms enregistres)"
    //   d) attendre le silence : lire des échantillons jusqu'à en avoir QUIET_SAMPLES
    //      D'AFFILÉE sous TRIGGER (remettre le compteur à 0 si un échantillon dépasse),
    //      puis afficher "En ecoute..."
    //      Pourquoi ? Sans ça, la fin du mot (ou son écho) déclencherait tout de suite
    //      un deuxième enregistrement. En partie 4, il serait appris comme le mot suivant !
    //
    #[idle(local = [dfsdm, led_red, led_yellow], shared = [training_mode])]
    fn idle(mut cx: idle::Context) -> ! {
        // Étape 4 : création du lecteur de microphone
        let mut mic = Microphone::new();
        // Étape 5 : tampon d'enregistrement d'un mot (INPUT_SIZE échantillons, pas de malloc)
        let mut buffer = [0i32; INPUT_SIZE];

        // Étape 4 : stabilisation du filtre du microphone (~8 000 lectures, ~1 s),
        // les deux LED allumées pendant ce temps
        cx.local.led_red.set_high();
        cx.local.led_yellow.set_high();
        for _ in 0..8000 {
            mic.read(cx.local.dfsdm);
        }
        cx.local.led_red.set_low();
        cx.local.led_yellow.set_low();

        // Étape 4 : message de mode au démarrage
        println!("MODE : inference (bouton bleu pour entrainer)");
        println!("En ecoute...");
        // Étape 4 : mode précédent, pour détecter un changement
        let mut last_mode = false;

        loop {
            // Étape 4 : lecture du mode (lock) ; si changé : message + LED jaune
            let mode = cx.shared.training_mode.lock(|m| *m);
            if mode != last_mode {
                last_mode = mode;
                if mode {
                    println!("MODE : entrainement");
                    cx.local.led_yellow.set_high();
                } else {
                    println!("MODE : inference");
                    cx.local.led_yellow.set_low();
                }
                println!("En ecoute...");
            }

            // Étape 5 : détection (amplitude > TRIGGER), sinon on recommence la boucle
            let (_, amplitude) = mic.read(cx.local.dfsdm);
            if amplitude < TRIGGER {
                continue;
            }

            // Étape 5 : enregistrement = historique (PRE_TRIGGER échantillons d'avant)...
            mic.copy_history(&mut buffer[..PRE_TRIGGER]);
            // Étape 5 : ... + 1 600 nouveaux échantillons
            for x in buffer[PRE_TRIGGER..].iter_mut() {
                let (valeur, _) = mic.read(cx.local.dfsdm);
                *x = valeur;
            }
            println!("Mot entendu (250 ms enregistres)");

            // Étape 6 : attendre QUIET_SAMPLES échantillons silencieux d'affilée
            // (compteur remis à 0 dès qu'un échantillon dépasse TRIGGER)
            let mut quiet: u32 = 0;
            while quiet < QUIET_SAMPLES {
                let (_, amplitude) = mic.read(cx.local.dfsdm);
                if amplitude < TRIGGER {
                    quiet += 1;
                } else {
                    quiet = 0;
                }
            }
            println!("En ecoute...");
        }
    }

    // TODO 3 : l'interruption du bouton bleu.
    //
    // Le bouton est sur PC13, donc sur la ligne EXTI 13 : quel vecteur d'interruption ?
    // (même chose que dans le projet feu de circulation)
    //   a) effacer le drapeau d'interruption du bouton (sinon elle se redéclenche sans fin)
    //   b) anti-rebond : un appui provoque souvent plusieurs fronts en quelques
    //      millisecondes. Ignorer l'interruption si le dernier appui pris en compte
    //      date de moins de DEBOUNCE_CYCLES cycles (`DWT::cycle_count()`).
    //      Astuce : le vumètre de la partie 1 (temp_micro_partie1/src/main.rs) mesure
    //      déjà une durée de cette façon, avec `DWT::cycle_count()` et `wrapping_sub`
    //      (variable `window_start`) : regardez comment il s'y prend.
    //      Il faut retenir l'instant du dernier appui : une ressource locale
    //      déclarée directement dans l'attribut convient :
    //        local = [btn, last_press: u32 = 0]
    //   c) inverser `training_mode` (avec un lock)
    //
    // Étape 2 : tâche btn_isr liée au vecteur EXTI15_10 (PC13 = ligne EXTI 13)
    // Étape 3 : ressource locale last_press (instant du dernier appui pris en compte)
    #[task(binds = EXTI15_10, local = [btn, last_press: u32 = 0], shared = [training_mode])]
    fn btn_isr(mut cx: btn_isr::Context) {
        // Étape 2 : effacer le drapeau, sinon l'interruption se redéclenche sans fin
        cx.local.btn.clear_interrupt_pending_bit();

        // Étape 3 : anti-rebond : ignorer les fronts à moins de DEBOUNCE_CYCLES (200 ms)
        // du dernier appui pris en compte
        let now = DWT::cycle_count();
        if now.wrapping_sub(*cx.local.last_press) < DEBOUNCE_CYCLES {
            return;
        }
        *cx.local.last_press = now;

        // Étape 2 : bascule du mode (avec un lock)
        cx.shared.training_mode.lock(|m| *m = !*m);
    }
}
