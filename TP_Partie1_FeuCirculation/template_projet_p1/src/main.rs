#![no_main]
#![no_std]

use cortex_m::delay::Delay;
use stm32h7xx_hal::{pac, prelude::*};

use defmt::println;
use defmt_rtt as _;
use panic_probe as _;

/// Pas de la boucle principale : le bouton est lu toutes les 10 ms.
const PAS_MS: u32 = 10;
/// Demi-période du clignotement jaune en mode dérangement.
const CLIGNOTEMENT_MS: u32 = 500;

/// Les trois états du cycle normal d'un feu.
#[derive(Clone, Copy, PartialEq, defmt::Format)]
enum Feu {
    Vert,
    Jaune,
    Rouge,
}

impl Feu {
    /// Durée de chaque état (cycle accéléré).
    fn duree_ms(self) -> u32 {
        match self {
            Feu::Vert => 3000,
            Feu::Jaune => 1000,
            Feu::Rouge => 3000,
        }
    }

    /// Cycle normal : vert -> jaune -> rouge -> vert.
    fn suivant(self) -> Feu {
        match self {
            Feu::Vert => Feu::Jaune,
            Feu::Jaune => Feu::Rouge,
            Feu::Rouge => Feu::Vert,
        }
    }
}

#[cortex_m_rt::entry]
fn main() -> ! {
    // BEGIN CONFIG : DO NOT EDIT OR REMOVE
    let dp = pac::Peripherals::take().expect("Cannot take peripherals");
    let cp = cortex_m::Peripherals::take().expect("Cannot take core peripherals");

    let pwrcfg = dp.PWR.constrain().freeze();

    let ccdr = dp.RCC.constrain().freeze(pwrcfg, &dp.SYSCFG);
    // END CONFIG

    let gpiob = dp.GPIOB.split(ccdr.peripheral.GPIOB);
    let gpioc = dp.GPIOC.split(ccdr.peripheral.GPIOC);
    let gpioe = dp.GPIOE.split(ccdr.peripheral.GPIOE);

    let mut led_green = gpiob.pb0.into_push_pull_output();
    let mut led_yellow = gpioe.pe1.into_push_pull_output();
    let mut led_red = gpiob.pb14.into_push_pull_output();

    // Bouton utilisateur (bleu) : état haut quand il est enfoncé.
    let bouton = gpioc.pc13.into_floating_input();

    // Delay de cortex-m : SysTick cadencé par l'horloge du processeur.
    // Celui de stm32h7xx-hal suppose une puce révision Y ; sur nos cartes
    // (révision V), ses délais durent 8 fois trop longtemps.
    let mut delay = Delay::new(cp.SYST, ccdr.clocks.c_ck().raw());

    let mut feu = Feu::Rouge;
    let mut ecoule_ms: u32 = 0;
    let mut derangement = false;
    println!("Demarrage : {}", feu);

    loop {
        if bouton.is_high() {
            // Mode dérangement : seul le jaune clignote.
            if !derangement {
                derangement = true;
                ecoule_ms = 0;
                println!("Derangement");
            }
            led_green.set_low();
            led_red.set_low();
            if (ecoule_ms / CLIGNOTEMENT_MS) % 2 == 0 {
                led_yellow.set_high();
            } else {
                led_yellow.set_low();
            }
        } else {
            if derangement {
                // Bouton relâché : on repart du rouge, l'état le plus sûr.
                derangement = false;
                feu = Feu::Rouge;
                ecoule_ms = 0;
                println!("Fin du derangement : {}", feu);
            } else if ecoule_ms >= feu.duree_ms() {
                feu = feu.suivant();
                ecoule_ms = 0;
                println!("{}", feu);
            }

            // Une seule LED allumée : celle de l'état courant.
            if feu == Feu::Vert { led_green.set_high() } else { led_green.set_low() }
            if feu == Feu::Jaune { led_yellow.set_high() } else { led_yellow.set_low() }
            if feu == Feu::Rouge { led_red.set_high() } else { led_red.set_low() }
        }

        delay.delay_ms(PAS_MS);
        ecoule_ms += PAS_MS;
    }
}
