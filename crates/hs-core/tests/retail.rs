//! Private-input regression, explicitly opt-in. Expected outcomes here are
//! recorded emulator observations, not independent hardware certification.
use hs_core::{Machine,Images,Time,TimedInput,Input,Buttons};
#[test]
#[ignore="requires HS_FIRMWARE and HS_EEPROM; run tools/verify_retail.py"]
fn retail_boot_and_button_replay_are_partition_invariant(){
    let firmware=std::fs::read(std::env::var("HS_FIRMWARE").expect("HS_FIRMWARE")).unwrap();
    let eeprom=std::fs::read(std::env::var("HS_EEPROM").expect("HS_EEPROM")).unwrap();
    let mut a=Machine::new(Images{firmware:&firmware,eeprom:&eeprom,eeprom_status:0}).unwrap();let mut b=a.clone();
    let inputs=[TimedInput{at:Time::from_micros(4_500_000),input:Input::Buttons(Buttons{center:true,left:false,right:false})},
        TimedInput{at:Time::from_micros(4_750_000),input:Input::Buttons(Buttons::RELEASED)}];
    let end=Time::from_micros(5_000_000);let mut x=Vec::new();let mut y=Vec::new();
    a.run_until(end,&inputs,&mut x).unwrap();
    let(mut micros,mut cursor,mut rng)=(0u64,0,987654321u32);
    while micros<5_000_000 {
        rng=rng.wrapping_mul(1664525).wrapping_add(1013904223);micros=(micros+1+u64::from(rng%17000)).min(5_000_000);
        let horizon=Time::from_micros(micros);let mut stop=cursor;while stop<inputs.len()&&inputs[stop].at<horizon{stop+=1;}
        cursor+=b.run_until(horizon,&inputs[cursor..stop],&mut y).unwrap().inputs_consumed;
    }
    assert_eq!(x,y,"complete product event histories differ");assert_eq!(a,b,"causal machine state differs");
    assert!(a.display_enabled());assert!(a.interrupt_entries()>0);assert!(a.ssu_counts().0>1000);
    let mut pixels=[0;6144];a.display(&mut pixels);assert!(pixels.iter().any(|v|*v!=0));
    let snapshot=a.snapshot();let mut c=Machine::from_snapshot(&snapshot);x.clear();y.clear();
    a.run_until(Time::from_micros(6_000_000),&[],&mut x).unwrap();c.run_until(Time::from_micros(6_000_000),&[],&mut y).unwrap();
    assert_eq!(x,y);assert_eq!(a,c);
}
