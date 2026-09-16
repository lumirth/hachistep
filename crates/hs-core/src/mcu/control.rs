//! MCU operating modes, clock/module controls, and external interrupt latches.
//! Invalid mode combinations stop explicitly; there is no product-level wake
//! shortcut based on a button name or retail firmware address.
use crate::{error::Error, time::{Duration, Time}};
use super::clocks::{Clocks, Tap};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Active, Subactive, Sleep, Subsleep, Watch, Standby }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Control {
    pub sys1:u8,pub sys2:u8,pub osc:u8,pub gate1:u8,pub gate2:u8,
    pub iegr:u8,pub ien1:u8,pub ien2:u8,pub irr1:u8,pub irr2:u8,
    pub mode:Mode,
    irq_levels:[Option<bool>;2],
}
impl Default for Control {
    fn default()->Self {Self {sys1:3,sys2:0xf0,osc:0,gate1:3,gate2:4,
        iegr:0,ien1:0,ien2:0,irr1:0,irr2:0,mode:Mode::Active,irq_levels:[None;2]}}
}
impl Control {
    pub fn handles(a:u16)->bool {matches!(a,0xfff0..=0xfff7|0xfffa|0xfffb)}
    pub fn read(&self,a:u16)->u8 {match a {
        0xfff0=>self.sys1,0xfff1=>self.sys2,0xfff2=>self.iegr,0xfff3=>self.ien1,
        0xfff4=>self.ien2,0xfff5=>self.osc,0xfff6=>self.irr1,0xfff7=>self.irr2,
        0xfffa=>self.gate1,_=>self.gate2,
    }}
    pub fn write(&mut self,a:u16,v:u8)->Result<(),Error> {
        match a {
            0xfff0=>self.sys1=v,
            0xfff1=>self.sys2=v|0xe0,
            0xfff2=>self.iegr=v&0xa3,
            0xfff3=>self.ien1=v&0x87,
            0xfff4=>self.ien2=v&0x45,
            0xfff5=>{
                if v&0xc0!=0 {return Err(Error::Unsupported{component:"clocks",
                    detail:"watch oscillator stop/external source switch is not implemented",address:a});}
                self.osc=v&0xe2;
            }
            0xfff6=>self.irr1&=v&7,
            0xfff7=>self.irr2&=v&0x45,
            0xfffa=>self.gate1=v&0x57,
            0xfffb=>self.gate2=v&0x7e,
            _=>return Err(Error::Unmapped{address:a,write:true,width:1}),
        }
        Ok(())
    }
    pub fn pins(&mut self,levels:[Option<bool>;2]) {
        for i in 0..2 {
            if let (Some(old),Some(new))=(self.irq_levels[i],levels[i]) {
                let rising=self.iegr&(1<<i)!=0;
                if old!=new && new==rising {self.irr1|=1<<i;}
            }
        }
        self.irq_levels=levels;
    }
    pub fn main_running(&self)->bool {matches!(self.mode,Mode::Active|Mode::Sleep)}
    pub fn sub_running(&self)->bool {matches!(self.mode,Mode::Subactive|Mode::Subsleep)}
    pub fn sleeping(&self)->bool {matches!(self.mode,Mode::Sleep|Mode::Subsleep|Mode::Watch|Mode::Standby)}
    fn select_clock(&self,now:Time,c:&mut Clocks)->Result<(),Error> {
        if matches!(self.mode,Mode::Subactive|Mode::Subsleep) {
            let divide=[8,4,2,1][usize::from(self.sys2&3)];
            c.set_system(now,c.frequencies.watch_hz,divide)
        } else {
            let divide=if self.sys2&4!=0 {[8,16,32,64][usize::from(self.sys1&3)]}else{1};
            c.set_system(now,c.frequencies.main_hz,divide)
        }
    }
    /// Return true for the architected direct-transition exception (vector 13).
    pub fn sleep(&mut self,now:Time,c:&mut Clocks)->Result<bool,Error> {
        let standby=self.sys1&0x80!=0;let sub=self.sys1&8!=0;
        let watch=self.sys1&4!=0;let direct=self.sys2&8!=0;
        if direct {
            self.mode=if standby&&watch&&sub {Mode::Subactive}
                else if !sub {Mode::Active}
                else{return Err(Error::Unsupported{component:"power",detail:"prohibited direct-transition combination",address:0xfff0});};
            self.select_clock(now,c)?;return Ok(true);
        }
        self.mode=if standby {if watch{Mode::Watch}else{Mode::Standby}}
            else if self.sub_running() {Mode::Subsleep}else{Mode::Sleep};
        Ok(false)
    }
    pub fn wake(&mut self,now:Time,c:&mut Clocks)->Result<Duration,Error> {
        let old=self.mode;
        self.mode=match old {
            Mode::Sleep=>Mode::Active,Mode::Subsleep=>Mode::Subactive,
            Mode::Watch if self.sys1&8!=0=>Mode::Subactive,
            Mode::Watch|Mode::Standby=>Mode::Active,m=>m,
        };
        self.select_clock(now,c)?;
        if matches!(old,Mode::Watch|Mode::Standby)&&self.mode==Mode::Active {
            let edges=[8192,16384,1024,2048,4096,256,512,16][usize::from(self.sys1>>4&7)];
            let end=c.after(now,edges,Tap::system(1))?;
            Ok(end.duration_since(now).unwrap())
        }else{Ok(Duration::ZERO)}
    }
    pub fn synchronize_clock(&self,now:Time,c:&mut Clocks)->Result<(),Error> {self.select_clock(now,c)}
}
