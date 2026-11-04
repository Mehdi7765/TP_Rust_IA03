#![no_main]
#![no_std]

use stm32h7xx_hal::{
    delay::DelayFromCountDownTimer,
    gpio::{Edge, ExtiPin, Input, Output, PB0, PB14, PC13, PE1, PushPull},
    pac,
    prelude::*,
    timer::Timer,
};

use defmt::println;
use defmt_rtt as _;
use panic_probe as _;

#[rtic::app(device = pac, peripherals = true)]
mod app {
    use super::*;

    // Shared resources: accessible from multiple tasks, but only through
    // a `.lock(...)` call that ensures safe concurrent access.
    #[shared]
    struct Shared {
        light_failed: bool,
    }

    #[local]
    struct Local {
        delay: DelayFromCountDownTimer<Timer<pac::TIM2>>,
        led_green: PB0<Output<PushPull>>,
        led_yellow: PE1<Output<PushPull>>,
        led_red: PB14<Output<PushPull>>,
        btn: PC13<Input>,
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local) {
        let mut dp = cx.device;

    // BEGIN CONFIG : DO NOT EDIT OR REMOVE
        // Constrain and Freeze power
        let pwrcfg = dp.PWR.constrain().freeze();

        // Constrain and Freeze clock
        let ccdr = dp.RCC.constrain().freeze(pwrcfg, &dp.SYSCFG);

        // Acquire GPIO peripherals
        let gpiob = dp.GPIOB.split(ccdr.peripheral.GPIOB);
        let gpioc = dp.GPIOC.split(ccdr.peripheral.GPIOC);
        let gpioe = dp.GPIOE.split(ccdr.peripheral.GPIOE);

        // LED setup
        let led_green = gpiob.pb0.into_push_pull_output();
        let led_yellow = gpioe.pe1.into_push_pull_output();
        let led_red = gpiob.pb14.into_push_pull_output();
    // END CONFIG

        // Bouton PC13 configuré comme source d'interruption (EXTI ligne 13).
        let mut btn = gpioc.pc13.into_input();
        // a) Relie la ligne EXTI 13 au port C (SYSCFG choisit le port).
        btn.make_interrupt_source(&mut dp.SYSCFG);
        // b) Déclenche sur les deux fronts : appui (montant) ET relâchement
        //    (descendant), pour savoir si le bouton est maintenu ou non.
        btn.trigger_on_edge(&mut dp.EXTI, Edge::RisingFalling);
        // c) Autorise l'interruption sur cette ligne.
        btn.enable_interrupt(&mut dp.EXTI);

    // BEGIN CONFIG : DO NOT EDIT OR REMOVE
        // Delay setup
        let delay = DelayFromCountDownTimer::new(dp.TIM2.timer(
            1.kHz(),
            ccdr.peripheral.TIM2,
            &ccdr.clocks,
        ));

        (
            Shared {
                light_failed: false,
            },
            Local {
                delay,
                led_green,
                led_yellow,
                led_red,
                btn,
            },
        )
    }
    // END CONFIG

    /// Boucle principale : pilote les LEDs.
    /// Elle ne lit plus le bouton : elle lit `light_failed`, mis à jour par
    /// l'interruption `btn_isr`.
    #[idle(local = [delay, led_green, led_yellow, led_red], shared = [light_failed])]
    fn idle(mut cx: idle::Context) -> ! {
        // 0 = vert, 1 = jaune, 2 = rouge
        let mut etape = 0;

        loop {
            // Lecture de la ressource partagée : obligatoirement via lock.
            let failed = cx.shared.light_failed.lock(|lf| *lf);

            if failed {
                // Dérangement : jaune clignotant (une période par tour).
                cx.local.led_green.set_low();
                cx.local.led_red.set_low();
                cx.local.led_yellow.set_high();
                cx.local.delay.delay_ms(500u16);
                cx.local.led_yellow.set_low();
                cx.local.delay.delay_ms(500u16);
            } else if etape == 0 {
                println!("Vert");
                cx.local.led_red.set_low();
                cx.local.led_yellow.set_low();
                cx.local.led_green.set_high();
                cx.local.delay.delay_ms(3000u16);
                etape = 1;
            } else if etape == 1 {
                println!("Jaune");
                cx.local.led_green.set_low();
                cx.local.led_red.set_low();
                cx.local.led_yellow.set_high();
                cx.local.delay.delay_ms(1000u16);
                etape = 2;
            } else {
                println!("Rouge");
                cx.local.led_green.set_low();
                cx.local.led_yellow.set_low();
                cx.local.led_red.set_high();
                cx.local.delay.delay_ms(3000u16);
                etape = 0;
            }
        }
    }

    /// Handler d'interruption du bouton, appelé par le matériel à chaque
    /// appui ou relâchement. PC13 = ligne EXTI 13, regroupée avec les
    /// lignes 10 à 15 dans le vecteur EXTI15_10.
    #[task(binds = EXTI15_10, local = [btn], shared = [light_failed])]
    fn btn_isr(mut cx: btn_isr::Context) {
        // a) Niveau actuel : haut = appuyé, bas = relâché.
        let appuye = cx.local.btn.is_high();

        // b) Mise à jour de la ressource partagée.
        cx.shared.light_failed.lock(|lf| *lf = appuye);

        // c) Acquitte l'interruption, sinon elle se redéclenche en boucle.
        cx.local.btn.clear_interrupt_pending_bit();

        // d) Trace dans la console RTT.
        if appuye {
            println!("ISR : front montant -> bouton appuye, derangement");
        } else {
            println!("ISR : front descendant -> bouton relache, cycle normal");
        }
    }
}
