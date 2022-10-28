use std::fmt;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::os::unix::prelude::FileExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{sleep, Duration};
extern crate derive_more;
use circular_queue::{CircularQueue, Iter};
use derive_more::Display;

const EMBEDDED_CONTROL_SYS_FILE: &str = "/sys/kernel/debug/ec/ec0/io";
const POLLING_INTERVAL: u64 = 5000;

const GPU_CONTROL_REGISTER: u64 = 0x89;
const GPU_TEMPERATURE_REGISTER: u64 = 0xb7;
const GPU_ACQUIRE_CONTROL: u8 = 0x04;
const GPU_RELEASE_CONTROL: u8 = 0x12;
const GPU_SPEED_CONTROL_REGISTER: u64 = 0xb7;

const CPU_CONTROL_REGISTER: u64 = 0xf4;
const CPU_TEMPERATURE_REGISTER: u64 = 0x58;
const CPU_ACQUIRE_CONTROL: u8 = 0x02;
const CPU_RELEASE_CONTROL: u8 = 0x00;
const CPU_SPEED_CONTROL_REGISTER: u64 = 0xf4;

static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);

#[derive(Display, Debug, PartialEq)]
struct TemperatureParseError;

impl std::error::Error for TemperatureParseError {}

#[derive(Display, Clone)]
#[display(fmt = "{}°", _0)]
struct Temperature(u8);

impl Temperature {
    fn from_milli_c(s: &str) -> Result<Temperature, TemperatureParseError> {
        s.parse::<u64>()
            .map_err(|_| TemperatureParseError)
            .map(|t| Temperature((t / 1000) as u8))
    }
}

use std::str::FromStr;
impl FromStr for Temperature {
    type Err = TemperatureParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse::<u8>()
            .map_err(|_| TemperatureParseError)
            .map(|t| Temperature(t))
    }
}

impl fmt::Debug for Temperature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(format!("{}°", self.0).as_str())
    }
}

use std::convert::TryFrom;
impl TryFrom<String> for Temperature {
    type Error = TemperatureParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

struct EcFanSpeedCommands {
    _0_pc: u8,
    _25_pc: u8,
    _50_pc: u8,
    _75_pc: u8,
    _100_pc: u8,
}

const CPU_FAN_SPEED_COMMANDS: EcFanSpeedCommands = EcFanSpeedCommands {
    _0_pc: 0x9,
    _25_pc: 0xa,
    _50_pc: 0x3d,
    _75_pc: 0x42,
    _100_pc: 0x47,
};

const GPU_FAN_SPEED_COMMANDS: EcFanSpeedCommands = EcFanSpeedCommands {
    _0_pc: 0x38,
    _25_pc: 0x40,
    _50_pc: 0x48,
    _75_pc: 0x50,
    _100_pc: 0x58,
};

struct HoldEcFanControl {
    control_register_offset: u64,
    release_control_value: u8,
}

impl Drop for HoldEcFanControl {
    fn drop(&mut self) {
        write_to_ec_register(self.control_register_offset, self.release_control_value).unwrap()
    }
}

impl HoldEcFanControl {
    pub fn new(
        control_register_offset: u64,
        acquire_control_value: u8,
        release_control_value: u8,
    ) -> io::Result<HoldEcFanControl> {
        write_to_ec_register(control_register_offset, acquire_control_value)?;
        Ok(HoldEcFanControl {
            control_register_offset,
            release_control_value,
        })
    }
}

fn get_temp_token_from_nvidia_smi_out(output: &str) -> Option<&str> {
    output.strip_suffix('\n')?.split_whitespace().last()
}

fn parse_temp_from_nvidia_smi_out(output: &str) -> Result<Temperature, TemperatureParseError> {
    match get_temp_token_from_nvidia_smi_out(output) {
        Some(s) => Temperature::from_str(s),
        None => Err(TemperatureParseError),
    }
}

fn read_i7_cpu_temp_from_file() -> std::io::Result<String> {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone8/temp")
}

fn read_i7_cpu_temp() -> Result<Temperature, TemperatureParseError> {
    let temp_s = read_i7_cpu_temp_from_file().unwrap();
    let temp = temp_s.strip_suffix('\n').unwrap();
    Temperature::from_milli_c(temp)
}

fn read_nvidia_gpu_temp() -> Result<Temperature, TemperatureParseError> {
    let output = Command::new("nvidia-smi")
        .args(&["stats", "-d", "temp", "-c", "1"])
        .output()
        .unwrap();

    let output = std::str::from_utf8(&output.stdout).unwrap();
    parse_temp_from_nvidia_smi_out(output)
}

fn write_to_ec_register(register_offset: u64, command: u8) -> io::Result<()> {
    let mut f = File::create(EMBEDDED_CONTROL_SYS_FILE)?;
    f.write_at(&[command], register_offset)?;
    f.flush()
}

fn read_from_ec_register(register_offset: u64) -> io::Result<u8> {
    let mut buf = [0u8; 1];
    let f = File::open(EMBEDDED_CONTROL_SYS_FILE)?;
    f.read_exact_at(&mut buf, register_offset)?;
    Ok(buf[0])
}

fn set_gpu_fan_speed(speed: u8) -> io::Result<()> {
    write_to_ec_register(GPU_SPEED_CONTROL_REGISTER, speed)
}

fn set_cpu_fan_speed(speed: u8) -> io::Result<()> {
    write_to_ec_register(CPU_SPEED_CONTROL_REGISTER, speed)
}

fn pid_controller(
    target: f64,
    temperature_history: Iter<Temperature>,
    polling_interval: u64,
    proportional_gain: f64,
    integral_gain: f64,
    derivative_gain: f64,
) -> f64 {
    let error_vals: Vec<f64> = temperature_history.map(|x| x.0 as f64 - target).collect();
    if error_vals.len() < 1 {
        return 0.0;
    }
    let latest_err = error_vals[0].clone();
    if error_vals.len() < 2 {
        return proportional_gain * latest_err;
    }
    let previous_err = error_vals[1].clone();
    let integral = error_vals.into_iter().sum::<f64>();
    let derivative = (latest_err - previous_err) / polling_interval as f64 as f64;
    println!(
        "Proportional coeff: {}, integral coeff: {}, derivative coeff: {}",
        proportional_gain * latest_err,
        integral_gain * integral,
        derivative_gain * derivative
    );
    proportional_gain * latest_err + integral_gain * integral + derivative_gain * derivative
}

fn map_gain_to_gpu_fan_speed(gain: f64) -> u8 {
    match gain {
        g if g < 15.0 => 0x38,
        g if g < 25.0 => 0x40,
        g if g < 35.0 => 0x48,
        g if g < 45.0 => 0x50,
        _ => 0x58,
    }
}

fn map_gain_to_cpu_fan_speed(gain: f64) -> u8 {
    match gain {
        g if g < 10.0 => 0x30,
        g if g < 20.0 => 0x38,
        g if g < 30.0 => 0x40,
        g if g < 40.0 => 0x48,
        g if g < 50.0 => 0x50,
        g if g < 60.0 => 0x58,
        _ => 0x60,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ctrlc::set_handler(|| {
        SHOULD_EXIT.store(true, Ordering::Relaxed);
    })?;
    let _hold_gpu_fan_control = HoldEcFanControl::new(
        GPU_CONTROL_REGISTER,
        GPU_ACQUIRE_CONTROL,
        GPU_RELEASE_CONTROL,
    )?;
    let _hold_cpu_fan_control = HoldEcFanControl::new(
        CPU_CONTROL_REGISTER,
        CPU_ACQUIRE_CONTROL,
        CPU_RELEASE_CONTROL,
    )?;

    let mut gpu_temperature_history = CircularQueue::<Temperature>::with_capacity(10);
    let mut cpu_temperature_history = CircularQueue::<Temperature>::with_capacity(10);
    let mut last_gpu_fan_speed: u8 = 0x0;
    let mut next_gpu_fan_speed: u8;
    let mut last_cpu_fan_speed: u8 = 0x0;
    let mut next_cpu_fan_speed: u8;
    while !SHOULD_EXIT.load(Ordering::Relaxed) {
        gpu_temperature_history.push(read_nvidia_gpu_temp().unwrap());
        cpu_temperature_history.push(read_i7_cpu_temp().unwrap());
        let gpu_gain = pid_controller(
            60.0,
            gpu_temperature_history.iter(),
            POLLING_INTERVAL,
            0.5,
            0.1,
            POLLING_INTERVAL as f64 * 2.0,
        );
        let cpu_gain = pid_controller(
            60.0,
            cpu_temperature_history.iter(),
            POLLING_INTERVAL,
            1.0,
            0.1,
            POLLING_INTERVAL as f64 * 0.5,
        );

        next_gpu_fan_speed = map_gain_to_gpu_fan_speed(gpu_gain);
        if next_gpu_fan_speed != last_gpu_fan_speed {
            set_gpu_fan_speed(next_gpu_fan_speed)?;
            last_gpu_fan_speed = next_gpu_fan_speed;
        }
        next_cpu_fan_speed = map_gain_to_cpu_fan_speed(cpu_gain);
        if next_cpu_fan_speed != last_cpu_fan_speed {
            set_cpu_fan_speed(next_cpu_fan_speed)?;
            last_cpu_fan_speed = next_cpu_fan_speed;
        }

        println!("GPU Gain: {}", gpu_gain);
        println!("GPU Temperature history: {:?}", gpu_temperature_history);

        println!("CPU Gain: {}", cpu_gain);
        println!("CPU Temperature history: {:?}", cpu_temperature_history);
        sleep(Duration::from_millis(POLLING_INTERVAL)).await;
    }
    Ok(())
}
