use hs_core::{Acceleration,Buttons,Input,Time,TimedInput};
use std::error::Error;

/// Human-editable deterministic physical inputs. No firmware state is modified.
pub fn parse(text:&str)->Result<Vec<TimedInput>,Box<dyn Error>> {
    let mut result=Vec::new();let mut prior=None;let mut seen=0u8;
    for (line_no,line) in text.lines().enumerate(){
        let line=line.split('#').next().unwrap_or("").trim();if line.is_empty(){continue;}
        let p:Vec<_>=line.split(',').map(str::trim).collect();
        let fail=|message:&str|format!("timeline line {}: {message}",line_no+1);
        if p.len()<3{return Err(fail("expected time_us,kind,values...").into());}
        let at=Time::from_micros(p[0].parse::<u64>().map_err(|_|fail("invalid microsecond timestamp"))?);
        let bit=|s:&str|->Result<bool,Box<dyn Error>>{match s{"0"=>Ok(false),"1"=>Ok(true),_=>Err(fail("Boolean values must be 0 or 1").into())}};
        let (input,tag)=match (p[1],p.len()) {
            ("buttons",5)=>(Input::Buttons(Buttons{left:bit(p[2])?,center:bit(p[3])?,right:bit(p[4])?}),0),
            ("accel",5)=>(Input::Acceleration(Acceleration{x:p[2].parse()?,y:p[3].parse()?,z:p[4].parse()?}),1),
            ("supply",3)=>(Input::SupplyMillivolts(p[2].parse()?),2),
            ("ir",3)=>(Input::InfraredLevel(bit(p[2])?),3),
            ("reset",3)=>(Input::ResetPin(bit(p[2])?),4),
            _=>return Err(fail("unknown input kind or incorrect number of values").into()),
        };
        if let Some(old)=prior{if at<old{return Err(fail("timestamps must be ordered").into());}if at!=old{seen=0;}}
        if seen&(1<<tag)!=0{return Err(fail("duplicate input property at the same timestamp").into());}
        seen|=1<<tag;prior=Some(at);result.push(TimedInput{at,input});
    }
    Ok(result)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn valid_and_invalid_timelines(){assert_eq!(parse("# test\n0,buttons,0,1,0\n0,accel,0,0,1000000\n2,ir,1").unwrap().len(),3);
        assert!(parse("2,ir,1\n1,ir,0").is_err());assert!(parse("0,ir,1\n0,ir,0").is_err());assert!(parse("0,buttons,0,2,0").is_err());}
}
