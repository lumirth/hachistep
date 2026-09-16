pub mod clocks;
pub mod rtc;
pub mod ssu;
pub mod timer_b1;
pub mod timer_w;
pub mod watchdog;
pub mod gpio;
pub mod adc;
pub mod control;
pub mod sci;

use crate::{cpu::Width,error::Error,signals::Output,time::Time};
use clocks::{Clocks,Frequencies,Tap};
use control::{Control,Mode};
use gpio::Gpio;
use rtc::Rtc;
use ssu::Ssu;
use timer_b1::TimerB1;
use timer_w::TimerW;
use watchdog::Watchdog;
use adc::Adc;
use sci::Sci;
pub const FLASH_SIZE:usize=49_152;
pub const RAM_START:u16=0xf780;
pub const RAM_SIZE:usize=2048;

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Mcu {
    pub(crate) flash:Box<[u8;FLASH_SIZE]>,
    pub(crate) ram:[u8;RAM_SIZE],
    pub clocks:Clocks,
    pub control:Control,
    pub gpio:Gpio,
    pub rtc:Rtc,
    pub ssu:Ssu,
    pub sci:Sci,
    pub timer_b1:TimerB1,
    pub timer_w:TimerW,
    pub watchdog:Watchdog,
    pub adc:Adc,
}
impl Mcu {
    pub fn new(firmware:&[u8],frequencies:Frequencies)->Result<Self,Error> {
        if firmware.len()!=FLASH_SIZE{return Err(Error::ImageSize{name:"firmware",expected:FLASH_SIZE,actual:firmware.len()});}
        let flash=firmware.to_vec().into_boxed_slice().try_into().map_err(|_|Error::Internal("flash allocation shape"))?;
        let mut m=Self{flash,ram:[0;RAM_SIZE],clocks:Clocks::new(Time::ZERO,frequencies)?,control:Control::default(),
            gpio:Gpio::default(),rtc:Rtc::default(),ssu:Ssu::default(),sci:Sci::default(),
            timer_b1:TimerB1::default(),timer_w:TimerW::default(),watchdog:Watchdog::default(),adc:Adc::default()};
        m.apply_gates(Time::ZERO,&mut ())?;Ok(m)
    }
    pub fn firmware(&self)->&[u8;FLASH_SIZE]{&self.flash}
    pub fn ram(&self)->&[u8;RAM_SIZE]{&self.ram}
    pub fn reset_vector(&self)->u16{u16::from_be_bytes([self.flash[0],self.flash[1]])}
    /// Synchronize clocked counters at an actual effect boundary. The return
    /// flag requests an MCU reset; attached device owners are not reconstructed.
    pub fn sync(&mut self,now:Time)->Result<bool,Error> {
        self.rtc.sync(now,&self.clocks)?;
        if self.timer_b1.sync(now,&self.clocks){self.control.irr2|=4;}
        self.timer_w.sync(now,&self.clocks);
        Ok(self.watchdog.sync(now,&self.clocks))
    }
    pub fn apply_gates(&mut self,now:Time,out:&mut dyn Output)->Result<(),Error> {
        let standby=self.control.mode==Mode::Standby;
        let main=self.control.main_running();
        let sub=self.control.sub_running();
        self.rtc.set_gate(!standby&&self.control.gate1&1!=0&&
            (main||self.rtc.uses_watch()),now,&self.clocks);
        self.timer_b1.set_gate(!standby&&self.control.gate1&4!=0&&
            (main||self.timer_b1.uses_watch()),now,&self.clocks);
        self.timer_w.set_gate(!standby&&self.control.gate2&0x40!=0&&
            (main||self.timer_w.uses_watch()),now,&self.clocks);
        self.watchdog.set_gate(self.control.gate2&4!=0,now,&self.clocks);
        self.ssu.set_gate((main||sub)&&self.control.gate2&0x10!=0,now,&self.clocks)?;
        self.sci.set_gate((main||sub)&&self.control.gate1&0x40!=0,now,out);
        self.adc.set_gate(self.control.gate1&0x10!=0&&(main||(sub&&self.adc.uses_watch())),now)?;
        Ok(())
    }
    pub fn power_on(&mut self, now: Time, out: &mut dyn Output) -> Result<(), Error> {
        self.clocks = Clocks::new(now, self.clocks.frequencies)?;
        self.rtc = Rtc::default(); self.ram.fill(0);
        self.reset(now, false, out)
    }
    pub fn reset(&mut self,now:Time,watchdog:bool,out:&mut dyn Output)->Result<(),Error> {
        // RAM, flash, watch-source phase, RTC, and external chips survive an MCU
        // reset. Undefined MCU RAM is initialized only by cold construction.
        self.control=Control::default();self.control.synchronize_clock(now,&mut self.clocks)?;
        self.gpio.reset();self.ssu=Ssu::default();self.sci.set_gate(false,now,out);
        self.timer_b1=TimerB1::default();self.timer_w=TimerW::default();self.adc=Adc::default();
        self.watchdog.reset(watchdog,now,&self.clocks);self.apply_gates(now,out)
    }
    pub fn deadline(&self)->Result<Option<Time>,Error> {
        Ok([self.rtc.deadline(&self.clocks)?,self.timer_b1.deadline(&self.clocks)?,
            self.timer_w.deadline(&self.clocks)?,self.watchdog.deadline(&self.clocks)?,
            self.ssu.deadline(),self.sci.deadline(),self.adc.deadline()].into_iter().flatten().min())
    }
    pub fn interrupt(&self)->Option<u8> {
        let mut best=None;
        let mut push=|v:u8|{best=Some(best.map_or(v,|old:u8|old.min(v)));};
        let ext=self.control.irr1&self.control.ien1;
        if ext&1!=0{push(16);}if ext&2!=0{push(17);}if ext&4!=0{push(18);}
        if self.control.ien1&0x80!=0 {if let Some(v)=self.rtc.interrupt(){push(v);}}
        if self.watchdog.interrupt(){push(31);}
        let request=self.control.irr2&self.control.ien2;
        if request&1!=0{push(32);}if request&4!=0{push(33);}
        if self.ssu.interrupt(){push(34);}if self.timer_w.interrupt(){push(35);}
        if self.sci.interrupt(){push(37);}if request&0x40!=0{push(38);}
        best
    }
    pub fn is_memory(a:u16)->bool{a<0xc000||(RAM_START..=0xff7f).contains(&a)}
    pub fn native_word(a:u16)->bool{Self::is_memory(a)||matches!(a,0xf0f6|0xf0f8|0xf0fa|0xf0fc|0xf0fe|0xffbc)}
    pub fn access_states(a:u16,width:Width)->u64 {
        if Self::is_memory(a)||width==Width::Word {2}
        else if matches!(a,0xffb0..=0xffb3|0xffc0..=0xffff){2}else{3}
    }
    pub fn read8(&mut self,a:u16)->Result<u8,Error> {
        if a<0xc000{return Ok(self.flash[usize::from(a)]);}
        if (RAM_START..=0xff7f).contains(&a){return Ok(self.ram[usize::from(a-RAM_START)]);}
        if Gpio::handles(a){return Ok(self.gpio.read(a));}
        if Control::handles(a){return Ok(self.control.read(a));}
        if Sci::handles(a){return Ok(self.sci.read(a));}
        match a {
            0xf067..=0xf06d|0xf06f=>Ok(self.rtc.read(a)),
            0xf0d0=>Ok(self.timer_b1.read(a)),0xf0d1=>Ok(self.timer_b1.read(a)),
            0xf0e0..=0xf0e4|0xf0e9|0xf0eb=>Ok(self.ssu.read(a)),
            0xf0f0..=0xf0f5=>Ok(self.timer_w.read(a)),
            0xffb0..=0xffb3=>Ok(self.watchdog.read(a)),
            0xffbe|0xffbf=>Ok(self.adc.peek(a)),
            0xf020..=0xf023|0xf02b=>Ok(0),
            _=>Err(self.unimplemented(a,false,1)),
        }
    }
    pub fn read16(&mut self,a:u16)->Result<u16,Error> {
        let a=a&!1;
        if a<0xc000{return Ok(u16::from_be_bytes([self.flash[usize::from(a)],self.flash[usize::from(a)+1]]));}
        if (RAM_START..=0xff7e).contains(&a){let i=usize::from(a-RAM_START);return Ok(u16::from_be_bytes([self.ram[i],self.ram[i+1]]));}
        match a{0xf0f6|0xf0f8|0xf0fa|0xf0fc|0xf0fe=>Ok(self.timer_w.word(a)),
            0xffbc=>Ok(self.adc.result()),_=>Err(self.unimplemented(a,false,2))}
    }
    pub fn write8(&mut self,a:u16,v:u8,mov:bool,now:Time,out:&mut dyn Output)->Result<(),Error> {
        if (RAM_START..=0xff7f).contains(&a){self.ram[usize::from(a-RAM_START)]=v;return Ok(());}
        if a<0xc000{return Err(Error::Unsupported{component:"flash",detail:"flash program/erase sequencer is not implemented",address:a});}
        if Gpio::handles(a){return self.gpio.write(a,v);}
        if Control::handles(a){self.control.write(a,v)?;return self.apply_gates(now,out);}
        if Sci::handles(a){return self.sci.write(a,v,now,&self.clocks,out);}
        match a{
            0xf067..=0xf06d|0xf06f=>self.rtc.write(a,v,now,&self.clocks),
            0xf0d0=>self.timer_b1.write(a,v,now,&self.clocks),
            0xf0d1=>self.timer_b1.write(a,v,now,&self.clocks),
            0xf0e0..=0xf0e4|0xf0e9|0xf0eb=>self.ssu.write(a,v,now,&self.clocks),
            0xf0f0..=0xf0f5=>self.timer_w.write(a,v,now,&self.clocks),
            0xffb0..=0xffb3=>self.watchdog.write(a,v,mov,now,&self.clocks),
            0xffbe|0xffbf=>self.adc.write(a,v,now,&self.clocks),
            0xf020..=0xf023|0xf02b if v==0=>Ok(()),
            _=>Err(self.unimplemented(a,true,1)),
        }
    }
    pub fn write16(&mut self,a:u16,v:u16,now:Time)->Result<(),Error> {
        let a=a&!1;
        if (RAM_START..=0xff7e).contains(&a){let i=usize::from(a-RAM_START);self.ram[i..i+2].copy_from_slice(&v.to_be_bytes());return Ok(());}
        match a{0xf0f6|0xf0f8|0xf0fa|0xf0fc|0xf0fe=>{self.timer_w.write_word(a,v,now,&self.clocks);Ok(())},
            0xffbc=>Ok(()),_=>Err(self.unimplemented(a,true,2))}
    }
    fn unimplemented(&self,a:u16,write:bool,width:u8)->Error {
        let component=match a{
            0xf020..=0xf02b=>"flash",0xf078..=0xf07f=>"IIC2",0xf0dc..=0xf0de=>"comparators",
            0xff8c..=0xff8f|0xff92|0xff94..=0xff97=>"AEC",
            0xf0f6..=0xf0ff=>"Timer W byte-access",0xffbc..=0xffbd=>"ADC word-only access",
            _=>return Error::Unmapped{address:a,write,width},
        };
        Error::Unsupported{component,detail:"this hardware access is not implemented; see docs/STATUS.md",address:a}
    }
    pub fn peek8(&self,a:u16)->Result<u8,Error> {
        if a<0xc000{return Ok(self.flash[usize::from(a)]);}
        if (RAM_START..=0xff7f).contains(&a){return Ok(self.ram[usize::from(a-RAM_START)]);}
        if Gpio::handles(a){return Ok(self.gpio.read(a));}
        if Control::handles(a){return Ok(self.control.read(a));}
        if Sci::handles(a){return Ok(self.sci.peek(a));}
        match a{
            0xf067..=0xf06d|0xf06f=>Ok(self.rtc.read(a)),0xf0d0=>Ok(self.timer_b1.read(a)),0xf0d1=>Ok(self.timer_b1.read(a)),
            0xf0e0..=0xf0e4|0xf0e9|0xf0eb=>Ok(self.ssu.peek(a)),0xf0f0..=0xf0ff=>Ok(self.timer_w.peek(a)),
            0xffb0..=0xffb3=>Ok(self.watchdog.peek(a)),0xffbe|0xffbf=>Ok(self.adc.peek(a)),
            0xffbc=>Ok((self.adc.result()>>8)as u8),0xffbd=>Ok(self.adc.result()as u8),
            _=>Err(self.unimplemented(a,false,1)),
        }
    }
    pub fn delay(&self,now:Time,states:u64)->Result<Time,Error>{self.clocks.after(now,states,Tap::system(1))}
}
