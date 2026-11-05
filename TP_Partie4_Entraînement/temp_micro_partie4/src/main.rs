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
// PARTIE 4 : ENTRAÎNEMENT SUR LA CARTE
//
// Point de départ : la solution de la partie 3 (enregistrement + inférence).
// Objectif : en mode entraînement, chaque mot enregistré sert à UNE étape
// d'entraînement du réseau. Les classes sont demandées dans l'ordre 0, 1, 2, 0...
//
// Les TODO sont dans `idle` (TODO 1 et 2 : entraînement ; TODO 3 : probabilités
// en inférence). Quand tout marche :
//   1. bouton bleu -> MODE : entrainement
//   2. dire 15 à 20 fois les 3 mots, dans l'ordre demandé (regarder les LED)
//   3. bouton bleu -> MODE : inference, puis tester chaque mot
//
// Pour aller plus loin : une 4e classe "autre" pour répondre "non reconnu"
// (voir la présentation de la séance).
// ============================================================================

#![no_main]
#![no_std]

extern crate alloc;

// [PARTIE 4 - TODO 3] Vec (no_std -> crate alloc) pour les probabilités en pourcentage
use alloc::vec::Vec;

mod microphone;
mod model;
mod utils;

use burn::{
    module::AutodiffModule,
    nn::loss::CrossEntropyLossConfig,
    optim::{AdamConfig, GradientsParams, Optimizer},
    tensor::Tensor,
};
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
/// Taux d'apprentissage : une étape par mot, il ne doit pas être trop petit
/// (avec 1e-3, le réseau n'apprend presque rien en quelques dizaines de mots)
const LEARNING_RATE: f64 = 1e-2;
/// Appuis sur le bouton plus rapprochés que ça ignorés (rebonds) : 200 ms
const DEBOUNCE_CYCLES: u32 = CPU_HZ / 5;

/// En mode entraînement, les LED indiquent le mot attendu :
/// jaune = classe 0, rouge = classe 1, les deux = classe 2
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
        // Tas (heap) pour Burn : 480 Ko de la RAM (la pile est dans la DTCM, voir memory.x).
        // Une étape d'entraînement utilise jusqu'à ~311 Ko, dont des blocs de 32 Ko d'un seul
        // tenant : avec 384 Ko, la mémoire finit par se fragmenter et une allocation échoue.
        {
            use core::mem::MaybeUninit;
            const ALLOCATION_BUFFER_SIZE: usize = 480 * 1024;
            static mut ALLOCATION_BUFFER: [MaybeUninit<u8>; ALLOCATION_BUFFER_SIZE] =
                [MaybeUninit::uninit(); ALLOCATION_BUFFER_SIZE];
            unsafe {
                ALLOCATOR.init(&raw mut ALLOCATION_BUFFER as usize, ALLOCATION_BUFFER_SIZE);
            }
        }

        // Réseau : 4 puis 8 filtres. Avec 2/3 (valeurs par défaut) il apprend moins bien,
        // avec 8/16 il ne tient plus en mémoire pendant l'entraînement.
        let device = burn::backend::ndarray::NdArrayDevice::default();
        let mut model: Model<ModelBackend> = ModelConfig::new(INPUT_SIZE, CLASSES)
            .with_conv1_channels_count(4)
            .with_conv2_channels_count(8)
            .init(&device);

        // TODO 1 : créer la fonction de perte et l'optimiseur.
        //
        //  - Perte : entropie croisée (classification). Regarder
        //    `CrossEntropyLossConfig` : comme les autres configurations Burn,
        //    on la crée avec `new()` puis on appelle `init(&device)`.
        //  - Optimiseur : Adam. Regarder `AdamConfig` ; son `init()` ne prend
        //    pas le device.
        //  L'optimiseur garde un état (moyennes des gradients) : il doit être
        //  modifiable (`mut`).
        //
        // [PARTIE 4 - TODO 1] Perte : entropie croisée, créée comme toute config Burn
        // (new() puis init(&device)). Le backend (ModelBackend) est déduit de son
        // utilisation plus bas dans loss.forward(...).
        let loss = CrossEntropyLossConfig::new().init(&device);
        // [PARTIE 4 - TODO 1] Optimiseur : Adam. init() sans device ; `mut` car Adam garde
        // un état interne (moyennes des gradients et de leurs carrés) d'une étape à l'autre.
        let mut optimizer = AdamConfig::new().init();

        let mut mic = Microphone::new();
        let mut buffer = [0i32; INPUT_SIZE];
        let mut training_class = 0;
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
                    training_class = 0;
                    println!("MODE : entrainement");
                    println!("Mot suivant : classe {}", training_class);
                    show_class(cx.local.led_yellow, cx.local.led_red, training_class);
                } else {
                    println!("MODE : inference");
                    cx.local.led_yellow.set_low();
                    cx.local.led_red.set_low();
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

            let start = DWT::cycle_count();
            let t: Tensor<ModelBackend, 1> = Tensor::from_floats(buffer, &device);
            if training_mode {
                // TODO 2 : une étape d'entraînement avec le mot qui vient d'être enregistré.
                //
                //  a) propagation avant : `model.forward(t)` donne 3 scores (Tensor à 1 dimension)
                //  b) perte : `loss.forward(scores, cibles)`
                //       - les scores doivent être de forme [batch, classes] = [1, 3] :
                //         regarder `unsqueeze`
                //       - la cible est la classe attendue, dans un Tensor d'entiers :
                //         `Tensor::from_ints([training_class], &device)`
                //  c) afficher la valeur de la perte : `into_scalar()`... mais il consomme
                //     le tenseur, dont on a encore besoin pour d) -> pensez à `clone()`
                //  d) rétropropagation : `backward()` sur la perte donne les gradients
                //  e) associer les gradients aux paramètres du modèle :
                //     `GradientsParams::from_grads(gradients, &model)`
                //  f) mise à jour : `optimizer.step(LEARNING_RATE, model, gradients)`
                //     renvoie le NOUVEAU modèle (le modèle est déplacé, pas modifié sur place)
                //  g) afficher "Classe entrainee N, perte = X (T ms)" : la durée se calcule
                //     avec `DWT::cycle_count()` et `start` (voir CPU_HZ)
                //  h) passer à la classe suivante (0 -> 1 -> 2 -> 0 ...), l'afficher
                //     ("Mot suivant : classe N") et l'indiquer avec les LED (`show_class`)
                // [PARTIE 4 - TODO 2] Étape d'entraînement sur le mot qui vient d'être dit.

                // a) Propagation avant en mode Autodiff (SANS valid()) : Burn construit
                //    le graphe de calcul nécessaire à la rétropropagation. -> [3] scores
                let scores = model.forward(t);

                // b) Perte : CrossEntropyLoss attend des scores [batch, classes] = [1, 3]
                //    (unsqueeze ajoute la dimension batch devant) et la classe attendue
                //    dans un tenseur d'entiers de forme [batch] = [1].
                let target = Tensor::from_ints([training_class], &device);
                let l = loss.forward(scores.unsqueeze::<2>(), target);

                // c) Valeur de la perte pour l'affichage : into_scalar() consomme le
                //    tenseur, or on en a encore besoin pour backward() -> clone().
                let loss_value: f32 = l.clone().into_scalar();

                // d) Rétropropagation : gradients de tous les tenseurs du graphe
                //    (le graphe est ensuite libéré).
                let grads = l.backward();

                // e) On ne garde que les gradients des paramètres du modèle.
                let grads = GradientsParams::from_grads(grads, &model);

                // f) Mise à jour des poids : step() prend le modèle (déplacé) et
                //    renvoie le NOUVEAU modèle, qu'on réaffecte à `model`.
                model = optimizer.step(LEARNING_RATE, model, grads);

                // g) Durée de l'étape : cycles écoulés depuis `start` convertis en ms
                //    (CPU_HZ / 1000 cycles par milliseconde). wrapping_sub gère le
                //    débordement du compteur 32 bits.
                let elapsed_ms = DWT::cycle_count().wrapping_sub(start) / (CPU_HZ / 1000);
                println!(
                    "Classe entrainee {}, perte = {} ({} ms)",
                    training_class, loss_value, elapsed_ms
                );

                // h) Classe suivante dans l'ordre 0 -> 1 -> 2 -> 0 ..., affichée sur la
                //    console et indiquée par les LED (jaune = 0, rouge = 1, les deux = 2).
                training_class = (training_class + 1) % CLASSES as i32;
                println!("Mot suivant : classe {}", training_class);
                show_class(cx.local.led_yellow, cx.local.led_red, training_class);
            } else {
                // Inférence : valid() donne le modèle sans Autodiff (pas de graphe de calcul gardé en mémoire)
                let r = model.valid().forward(t.inner());
                let scores = r.to_data().to_vec::<f32>().expect("Tensor data error");
                // [PARTIE 4] argmax consomme le tenseur : r.clone() pour pouvoir encore
                // l'utiliser dans softmax (TODO 3) juste en dessous.
                let class = r.clone().argmax(0).into_scalar();

                // TODO 3 : afficher aussi les probabilités, en pourcentage.
                //
                //  Les scores (logits) peuvent être négatifs ; softmax les transforme en
                //  probabilités (positives, de somme 1) :
                //    `burn::tensor::activation::softmax(tenseur, dimension)`
                //  - softmax consomme le tenseur : r est encore utilisé ensuite -> `r.clone()`
                //  - récupérer les valeurs avec `to_data().to_vec::<f32>()`, comme pour les scores
                //  - les convertir en pourcentages entiers (u32) : `iter().map(...).collect()`
                //    dans un `Vec<u32>` (`use alloc::vec::Vec;` en haut du fichier)
                //  - afficher par exemple :
                //    "Scores [...] -> probabilites [36, 38, 26] % -> classe predite 1 (38 %)"
                //  Comparer les probabilités avant et après l'entraînement.
                // [PARTIE 4 - TODO 3] Probabilités : softmax sur la dimension 0
                // (p_i = exp(s_i) / somme des exp(s_j)). softmax consomme le tenseur :
                // r n'est plus utilisé après, on peut le lui donner directement.
                let probs = burn::tensor::activation::softmax(r, 0)
                    .to_data()
                    .to_vec::<f32>()
                    .expect("Tensor data error");
                // Conversion en pourcentages entiers, arrondis au plus proche
                // (pas de f32::round en no_std : on ajoute 0,5 avant de tronquer).
                let percents: Vec<u32> = probs.iter().map(|p| (p * 100.0 + 0.5) as u32).collect();
                println!(
                    "Scores {} -> probabilites {} % -> classe predite {} ({} %)",
                    scores.as_slice(),
                    percents.as_slice(),
                    class,
                    percents[class as usize]
                );
                show_class(cx.local.led_yellow, cx.local.led_red, class as i32);
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
