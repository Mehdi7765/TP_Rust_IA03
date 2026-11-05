// ============================================================================
// DOCUMENTATION UTILE
//
//  Burn (livre + exemples) :          https://burn.dev/books/burn/
//  API de Burn :                      https://docs.rs/burn
//  API de la HAL (GPIO, EXTI...) :    https://docs.rs/stm32h7xx-hal
//  Livre RTIC (#[idle], #[task], lock) : https://rtic.rs/2/book/en/
//  Livre Rust :                       https://doc.rust-lang.org/book/
//
//  Toujours lancer avec : cargo run --release
// ============================================================================
//
// PARTIE 3 : LE RÉSEAU DE NEURONES ET L'INFÉRENCE
//
// Point de départ : la solution de la partie 2 (mode, enregistrement d'un mot).
// Objectif : créer le réseau avec Burn et, en mode inférence, afficher les
// 3 scores et la classe prédite pour chaque mot enregistré. Le réseau n'est
// pas encore entraîné (partie 4) : les prédictions sont donc au hasard, mais
// voir 3 scores s'afficher prouve que toutes les dimensions sont correctes.
//
// Les TODO sont dans `src/model.rs` (le réseau) et dans `idle` ci-dessous.
// ============================================================================

#![no_main]
#![no_std]

extern crate alloc;

mod microphone;
mod model;
mod utils;

use burn::{module::AutodiffModule, tensor::Tensor};
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

use embedded_alloc::LlffHeap as EmbeddedAllocator;

use microphone::{Microphone, PRE_TRIGGER};
use model::{Model, ModelBackend, ModelConfig};

// Allocateur global : Burn a besoin d'allouer de la mémoire (Vec, tenseurs...)
#[global_allocator]
static ALLOCATOR: EmbeddedAllocator = EmbeddedAllocator::empty();

/// Horloge du processeur (laissée par défaut par utils::init : HSI 64 MHz)
const CPU_HZ: u32 = 64_000_000;
/// Échantillons par enregistrement : 2000 à ~8 000 éch/s = 250 ms
const INPUT_SIZE: usize = 2000;
/// Nombre de mots (classes) à reconnaître
const CLASSES: usize = 3;
/// Amplitude (sans le décalage) qui déclenche un enregistrement.
/// Silence : < 20 000 ; voix : 50 000 à 1 000 000 ; claquement de mains : jusqu'à 7 000 000
const TRIGGER: u32 = 1 << 17;
/// Échantillons sous TRIGGER nécessaires avant de réécouter (~0,3 s)
const QUIET_SAMPLES: u32 = 2400;
/// Appuis sur le bouton plus rapprochés que ça ignorés (rebonds) : 200 ms
const DEBOUNCE_CYCLES: u32 = CPU_HZ / 5;

/// Affiche une classe avec les LED : jaune = classe 0, rouge = classe 1, les deux = classe 2
fn show_class(led_yellow: &mut PE1<Output<PushPull>>, led_red: &mut PB14<Output<PushPull>>, class: i32) {
    match class {
        0 => {
            led_yellow.set_high();
            led_red.set_low();
        }
        1 => {
            led_yellow.set_low();
            led_red.set_high();
        }
        _ => {
            led_yellow.set_high();
            led_red.set_high();
        }
    }
}

#[rtic::app(device = hal::pac, peripherals = true)]
mod app {
    use super::*;

    #[shared]
    struct Shared {
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
        let dp = cx.device;
        let mut cp = cx.core;

        // Compteur de cycles : anti-rebond du bouton et mesure des durées
        cp.DCB.enable_trace();
        cp.DWT.enable_cycle_counter();

        let (delay, led_red, led_yellow, btn, dfsdm) = utils::init(dp);

        (
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

    #[idle(local = [delay, led_red, led_yellow, dfsdm], shared = [training_mode])]
    fn idle(mut cx: idle::Context) -> ! {
        // TODO 1 : initialiser le tas (heap) de 480 Ko pour Burn.
        //
        // L'allocateur global ALLOCATOR est déclaré plus haut, mais il ne connaît
        // encore aucune zone mémoire : sans cette initialisation, la première
        // allocation de Burn provoque un panic "memory allocation ... failed".
        // Voir la diapositive "Allocation" du cours Rust : un tableau `static mut`
        // de `MaybeUninit<u8>`, puis `ALLOCATOR.init(adresse, taille)` dans un bloc `unsafe`.
        //
        // Attention : prendre 480 Ko, pas les 384 Ko de la diapositive. Pendant l'entraînement
        // (partie 4), Burn utilise jusqu'à ~311 Ko, dont des blocs de 32 Ko d'un seul tenant :
        // avec 384 Ko, la mémoire finit par se fragmenter et une allocation échoue après
        // une centaine de mots. 480 Ko est possible car la pile a été déplacée dans la
        // DTCM (voir memory.x) : la RAM de 512 Ko est presque entièrement libre pour le tas.

        // Étape 4 : initialisation du tas de 480 Ko (avant toute allocation de Burn)
        {
            use core::mem::MaybeUninit;
            const HEAP_SIZE: usize = 480 * 1024;
            static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
            // SAFETY : appelé une seule fois, avant la première allocation ; HEAP_MEM
            // n'est utilisé nulle part ailleurs.
            unsafe { ALLOCATOR.init(&raw mut HEAP_MEM as usize, HEAP_SIZE) }
        }

        // TODO 2 : créer le réseau (voir aussi les TODO de src/model.rs).
        //
        //  - le device : `burn::backend::ndarray::NdArrayDevice::default()`
        //  - le réseau : `ModelConfig::new(INPUT_SIZE, CLASSES)` puis `.init(&device)`,
        //    de type `Model<ModelBackend>`
        //  - utiliser 4 filtres pour la 1re convolution et 8 pour la 2e : la configuration
        //    générée par `#[derive(Config)]` fournit des méthodes `with_<nom_du_champ>(valeur)`.
        //    (Avec les valeurs par défaut 2/3, le réseau apprend moins bien ; avec 8/16,
        //    il ne tient plus en mémoire pendant l'entraînement.)
        //
        // let device = ...;
        // let model: Model<ModelBackend> = ...;

        // Étape 4 : création du réseau avec 4 puis 8 filtres
        let device = burn::backend::ndarray::NdArrayDevice::default();
        let model: Model<ModelBackend> = ModelConfig::new(INPUT_SIZE, CLASSES)
            .with_conv1_channels_count(4)
            .with_conv2_channels_count(8)
            .init(&device);

        let mut mic = Microphone::new();
        let mut buffer = [0i32; INPUT_SIZE];
        let mut previous_mode = false;

        // Laisser le filtre du microphone et l'estimation du décalage se stabiliser (~1 s)
        cx.local.led_yellow.set_high();
        cx.local.led_red.set_high();
        for _ in 0..8000 {
            mic.read(cx.local.dfsdm);
        }
        cx.local.led_yellow.set_low();
        cx.local.led_red.set_low();
        println!("MODE : inference (bouton bleu pour entrainer)");
        println!("En ecoute...");

        loop {
            // Changement de mode (demandé par le bouton)
            let training_mode = cx.shared.training_mode.lock(|training_mode| *training_mode);
            if training_mode != previous_mode {
                previous_mode = training_mode;
                if training_mode {
                    println!("MODE : entrainement");
                    cx.local.led_yellow.set_high();
                } else {
                    println!("MODE : inference");
                    cx.local.led_yellow.set_low();
                }
                println!("En ecoute...");
            }

            // Attente d'un son assez fort
            let (_, amplitude) = mic.read(cx.local.dfsdm);
            if amplitude < TRIGGER {
                continue;
            }

            // Enregistrement : les PRE_TRIGGER derniers échantillons, puis la suite du mot
            mic.copy_history(&mut buffer[..PRE_TRIGGER]);
            for sample in buffer[PRE_TRIGGER..].iter_mut() {
                *sample = mic.read(cx.local.dfsdm).0;
            }
            println!("Mot entendu ({} ms enregistres)", INPUT_SIZE as u32 / 8);

            if training_mode {
                println!("(l'entrainement sera fait en partie 4)");
            } else {
                // TODO 3 : inférence sur le mot enregistré.
                //
                //  a) convertir `buffer` en tenseur : `Tensor::from_floats(buffer, &device)`,
                //     de type `Tensor<ModelBackend, 1>`
                //  b) propagation avant SANS calcul de gradient : `model.valid()` donne une
                //     copie du réseau sans Autodiff, qui attend un tenseur sans Autodiff :
                //     `t.inner()`. (Avec `model.forward(t)`, Burn garderait en mémoire tout
                //     le graphe de calcul en attendant un `backward()` qui ne vient jamais.)
                //  c) afficher les 3 scores : `r.to_data().to_vec::<f32>()` renvoie un Result
                //     contenant un Vec ; `println!` sait afficher `vec.as_slice()`
                //  d) la classe prédite est l'indice du plus grand score : `argmax(0)`,
                //     puis `into_scalar()` pour obtenir un nombre
                //  e) afficher "Scores [...] -> classe predite N", montrer la classe avec
                //     `show_class` pendant 500 ms (`cx.local.delay.delay_ms(500u32)`),
                //     puis éteindre les LED

                // Étape 5 a) : le mot enregistré devient un tenseur [2000]
                let t: Tensor<ModelBackend, 1> = Tensor::from_floats(buffer, &device);
                // Étape 5 b) : valid() + inner() -> calcul sans Autodiff (pas de graphe gardé)
                let r = model.valid().forward(t.inner()); // 3 scores
                // Étape 5 c) : récupérer les 3 scores pour les afficher
                let scores = r.clone().to_data().to_vec::<f32>().unwrap();
                // Étape 5 d) : la classe prédite est l'indice du plus grand score
                let class = r.argmax(0).into_scalar() as i32;
                // Étape 5 e) : affichage, LED pendant 500 ms, puis extinction
                println!("Scores {} -> classe predite {}", scores.as_slice(), class);
                show_class(cx.local.led_yellow, cx.local.led_red, class);
                cx.local.delay.delay_ms(500u32);
                cx.local.led_yellow.set_low();
                cx.local.led_red.set_low();
            }

            // Attendre la fin du mot (et de son écho) avant de réécouter,
            // sinon la fin du mot serait enregistrée comme le mot suivant
            let mut quiet = 0;
            while quiet < QUIET_SAMPLES {
                let (_, amplitude) = mic.read(cx.local.dfsdm);
                quiet = if amplitude < TRIGGER { quiet + 1 } else { 0 };
            }
            println!("En ecoute...");
        }
    }

    #[task(binds = EXTI15_10, local = [btn, last_press: u32 = 0], shared = [training_mode])]
    fn btn_isr(mut cx: btn_isr::Context) {
        cx.local.btn.clear_interrupt_pending_bit();
        // Anti-rebond : ignorer les appuis trop rapprochés
        let now = DWT::cycle_count();
        if now.wrapping_sub(*cx.local.last_press) < DEBOUNCE_CYCLES {
            return;
        }
        *cx.local.last_press = now;
        cx.shared
            .training_mode
            .lock(|training_mode| *training_mode = !*training_mode);
    }
}
