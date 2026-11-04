// ============================================================================
// USEFUL DOCUMENTATION - bookmark these, you'll need them!
//
//  HAL API reference (ExtiPin, Edge, Timer, GPIO methods):
//      https://docs.rs/stm32h7xx-hal
//
//  HAL examples (exti_interrupt.rs is particularly relevant):
//      https://github.com/stm32-rs/stm32h7xx-hal/tree/master/examples
//
//  RTIC book — syntax for #[task], #[idle], .lock(), resource lists:
//      https://rtic.rs/1/book/en/
//
//  The Rust Book — general language syntax (enums, match, structs, etc.):
//      https://doc.rust-lang.org/book/
//
//  STM32H753 Reference Manual (interrupt vector table, peripheral details):
//      RM0433 (linked in the course slides)
// ============================================================================

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

        // TODO: configure the button (PC13) as an interrupt source.
        //
        // In part 1, you just read the pin's level in a loop. Now you need
        // the hardware to NOTIFY you when the button changes, rather than
        // having to check constantly. This is done by configuring the pin
        // as an EXTI (External Interrupt) source.
        //
        // You need to:
        //   a) create the pin as an input (gpioc.pc13.into_input())
        //   b) make it an interrupt source (look at the ExtiPin trait -
        //      which methods does it provide? what arguments do they need?
        //      you'll need access to dp.SYSCFG and dp.EXTI)
        //   c) choose which edge(s) to trigger on: should the interrupt
        //      fire only when pressed, only when released, or both? Think
        //      about what `idle` needs to know to behave correctly.
        //   d) enable the interrupt
        //
        // Hint: look at the `ExtiPin` trait in the stm32h7xx-hal docs,
        // and at the `Edge` enum variants.
        let mut btn = gpioc.pc13.into_input();
        // b) Relie la ligne EXTI 13 au port C (SYSCFG choisit le port).
        btn.make_interrupt_source(&mut dp.SYSCFG);
        // c) Les deux fronts : appui (montant) ET relâchement (descendant),
        //    pour qu'idle sache si le bouton est maintenu ou non.
        btn.trigger_on_edge(&mut dp.EXTI, Edge::RisingFalling);
        // d) Autorise l'interruption sur cette ligne.
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

    // TODO: implement `idle` - the main loop that drives the LEDs.
    //
    // This replaces your Part 1 `loop { ... }` inside `main()`. The logic
    // is the same - green -> yellow -> red cycle, or yellow blink when the
    // button is held - but with one important difference: the button pin
    // is no longer available here (it belongs to `btn_isr` below), so you
    // can't call `btn.is_high()` anymore. Instead, you need to read the
    // shared `light_failed` boolean that `btn_isr` updates for you.
    //
    // Reading a shared resource requires a lock:
    //   let value = cx.shared.some_resource.lock(|res| *res);
    //
    // Local resources are accessed directly:
    //   cx.local.led_green.set_high();
    //   cx.local.delay.delay_ms(1000u16);
    //
    // Don't forget:
    //   - the function signature must return `-> !` (never returns)
    //   - you need to declare which shared and local resources this task
    //     uses in the `#[idle(...)]` attribute
    //
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

    // TODO: implement `btn_isr` - the button interrupt handler.
    //
    // This function is called by the hardware every time the button's
    // state changes (assuming you configured the edges correctly above).
    // It needs to be bound to the right interrupt vector.
    //
    // PC13 is on EXTI line 13, which belongs to the EXTI10-15 group.
    // Look up the interrupt vector name for that group - it's used in the
    // `binds = ...` part of the task attribute.
    //
    // Inside the handler, you need to:
    //   a) figure out whether the button is currently pressed or released
    //      (the pin is available as a local resource - what method on an
    //      input pin tells you its current level?)
    //   b) update `light_failed` accordingly through a lock
    //   c) clear the interrupt pending bit on the pin, or the interrupt
    //      will keep re-triggering immediately and nothing else will run
    //      (what method on ExtiPin does this?)
    //   d) add some logging (println!) so you can see on the RTT console
    //      when interrupts fire - this will be very helpful for debugging
    //
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
