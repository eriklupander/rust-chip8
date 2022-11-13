#![allow(dead_code, non_snake_case)]


use std::sync::{Mutex, Arc};
use std::time::{Instant};
use std::{fs, thread, time};
use pixels::{Pixels, SurfaceTexture};

use winit::dpi::LogicalSize;
use winit::event::{Event, VirtualKeyCode};
use winit::event_loop::{EventLoop};
use winit::window::WindowBuilder;
use winit_input_helper::WinitInputHelper;

const MEM_OFFSET: i32 = 0x200;
const FONT_OFFSET: u16 = 0x50;

const MIN_DURATION: u128 = 1000; // Langhoffs guide recommends 700 ops/s, but I think that's on the slow side.
       
const FORCE_COSMAC_VIP: bool = false;

fn main() {

    println!("Welcome to Rust CHIP8!");
    
    // Init window / pixels
    let event_loop = EventLoop::new();
    
    let window = {
        let size = LogicalSize::new(640 as f64, 320 as f64);
        WindowBuilder::new()
            .with_title("Rust-CHIP8")
            .with_inner_size(size)
            .with_min_inner_size(size)
            .build(&event_loop)
            .unwrap()
    };

    let pixels = {
        let window_size = window.inner_size();
        let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
        Pixels::new(64, 32, surface_texture)
    }.unwrap();

    // set up shared ownership constructs for the "pixels" data.
    let screen = Arc::new(Mutex::new(pixels));
    let screenClone1 = Arc::clone(&screen);
    let screenClone2 = Arc::clone(&screen);

    let input = WinitInputHelper::new();
    let inputArc = Arc::new(Mutex::new(input));
    let inputClone1 = Arc::clone(&inputArc);
    let inputClone2 = Arc::clone(&inputArc);

    // Run the interpreter in a dedicated thread. Each "tick" of the event loop will process
    // a single instruction and, if necessary, updated the "pixels" data.
    thread::spawn(move||{
        // load ROM
        //let data = fs::read("./roms/IBM logo.ch8").expect("Unable to read file");
        //let data = fs::read("./roms/c8_test.c8").expect("Unable to read file");
        let data = fs::read("./roms/pong.ch8").expect("Unable to read file");

        // Init emulator with rom data
        let mut emul = initEmulator(data);
       
        // timer is used for updating delayTimer and soundTimer every time approx. 1/60th second has elapsed.
        let mut timer = Instant::now();
 
        loop {
            // keeps track of how many microseconds this instruction took, so we can pace the interpreter.
            let throttle = Instant::now();

            // Let emulator process one instruction
            {
                emul.run(&screenClone1, &inputClone1);
            }
            
            // update timers each time more than 16.6 ms (16666 microseconds) have passed, i.e. 60hz   
            if timer.elapsed().as_micros() > 16666 {
                if emul.delayTimer > 0 {
                    emul.delayTimer -= 1;
                }
                if emul.soundTimer > 0 {
                    emul.soundTimer -= 1;
                }
                timer = Instant::now();
            }

            // artificially slow down interpreter to run at approx 700 instructions per second
            if throttle.elapsed().as_micros() < MIN_DURATION {
                thread::sleep(time::Duration::from_micros((MIN_DURATION - throttle.elapsed().as_micros()) as u64 ));
            } 
        }
    
    });

    // Let the winit event-loop handle screen redraws.
    event_loop.run(move |event, _, _control_flow| {
       
        // Handle keystrokes including exit through ESC or clicking (x)
        {
            let mut keyStrokes = inputClone2.lock().unwrap();
            if keyStrokes.update(&event) {
                if keyStrokes.key_pressed(VirtualKeyCode::Escape) || keyStrokes.quit(){
                    println!("Exit requested");
                    _control_flow.set_exit();
                    return;
                }
            }
        }
        
        // Draw the current frame
        if let Event::RedrawRequested(_) = event {
            screenClone2.lock().unwrap().render().expect("do not fail");

             // We must tell the window to redraw. 
            window.request_redraw();
        }
    });

    

}

fn initEmulator(data: Vec<u8>) -> Emulator {
    let mut memory_array: [u8; 4096] = [0; 4096];

    // copy program into memory
    let mut i = MEM_OFFSET;
    for b in data.iter() {
        memory_array[i as usize] = *b;
        i += 1;
    }

    // copy font into memory
    let mut i = FONT_OFFSET;
    for b in FONT.iter() {
        memory_array[i as usize] = *b;
        i += 1;
    }

    let stack_array: [u16; 32] = [0; 32];
    let registers_array: [u8; 16] = [0; 16];

    let emul: Emulator = Emulator {
        memory: memory_array,
        stack: stack_array,
        stackFrame: -1,
        I: 0,
        registers: registers_array,
        pc: MEM_OFFSET as u16,
        delayTimer: 0x0,
        soundTimer: 0x0,
    };
    emul
}

struct Emulator {
    memory: [u8; 4096],  // Our 4kb of RAM
    stack: [u16; 32],    // The stack offers a max depth of 32 with 2 bytes per stack frame
    stackFrame: i8,      // current stack frame. Starts at -1 and is set to 0 on first use
    I: u16,              // represents Index register
    registers: [u8; 16], // represents the 16 1-byte registers
    pc: u16,             // Program counter, set it to the initial memory offset
    delayTimer: u8,      // represents the delay timer that's decremented at 60hz if > 0
    soundTimer: u8,      // represents the sound timer that's decremented at 60hz and plays a beep if > 0.
}

impl Emulator {

    // run runs a single CHP8 instruction.
    fn run(&mut self, pixels: &Arc<Mutex<Pixels>>, input: &Arc<Mutex<WinitInputHelper>>){

        // parse next instruction from memory, using the pc (program counter) value.
        let b = ((self.memory[self.pc as usize] as u16) << 8) | self.memory[self.pc as usize + 1] as u16;
        
        let b0 = (b & 0xFF00) >> 8 as u8;  // To get first byte, & the 8 leftmost bits which removes the 8 rightsmost, then shift by 8 to the right to make the u8 conversion contain the bits originally on the left.
        let b1 = (b & 0x00FF) as u8;        // To get the second byte, just & the 8 rightmost bits, which removes the leftmost bits. The remaining bits are already at the correct location so no need to shift before converting to u8.
        
        let instr = (b0 & 0xF0) >> 4 as u8;    // first nibble, the instruction. Keep 4 leftmost bits, then shift them to the right-hand side.
        let X = (b0 & 0x0F) as usize;        // second nibble, register lookup! Only keep rightmost bits.
        let Y = ((b1 & 0xF0) >> 4) as usize; // third nibble, register lookup! Keep leftmost bits, shift 4 to left.
        let N = b1 & 0x0F;                      // fourth nibble, 4 bit number
        let NN = b1;                            // NN = second byte
        let NNN = (b & 0x0FFF) as u16;         // NNN = second, third and fourth nibbles

        //print!("PC: {} B: {:#X} B0: {:#X} B1: {:#X}", self.pc, b, b0, b1);
        //println!(" Instr: {:#X} X: {:#X} Y: {:#X}", instr, X, Y);
        //println!(" N: {:#X} NN: {:#X} NNN: {:#X}", N, NN, NNN);
        
        self.pc += 2;

        // match the instruction
        match (instr, X, Y, N)  {
            // 0x00E0 Clear screen
            (0x0, 0x0, 0xE, 0x0) => println!("clear screen"),

            // 0x00EE Pop stack
            (0x0, 0x0 ,0xE, 0xE) => {
                self.pc = self.stack[self.stackFrame as usize]; // remember - this is actually the "parent" stack frame
                self.stackFrame -= 1;
            },

            // 0x1: Jump program counter to NNN
            (0x1, _, _, _) => {
                self.pc = NNN;
            }
            // 0x2: Subroutine: Push to stack, then set PC to NNN
            (0x2, _, _, _) => {
                self.stackFrame+=1;
                self.stack[self.stackFrame as usize] = self.pc; // store _current_ program counter in the NEXT stack frame.
                self.pc = NNN;
            }

            // 0x3: Skip if value in register X equals NN
            (0x3, _, _, _) => {
                // println!("ENTER - 3XNN - is value is register {}[{}] equal to {}", X, self.registers[X], NN);
                if self.registers[X] == NN {
                    self.pc += 2;
                }
            }

            // 0x4: Skip if value in register X not equals NN
            (0x4, _, _, _) => {
                // println!("ENTER - 4XNN - skip if register {}[{}] != {}", X, self.registers[X], NN);
                if self.registers[X] != NN {
                    self.pc += 2;
                }
            } 

            // 0x5: Skip if values in registers X and Y are equal
            (0x5, _, _, _) => {
                // println!("ENTER - 5XNN");
                if N == 0x0 && self.registers[X] == self.registers[Y] {
                    self.pc += 2
                }
            }
        
            // 0x6: Set register X to NN
            (0x6, _, _, _) => {
                // println!("set register {} to {}", X, NN);
                self.registers[X] = NN;
            }

            // 0x7: Add NN to register X
            (0x7, _, _, _) => {
                // print!("ENTER - 7XNN - add {} to register {} [{}]", NN, X, self.registers[X]);
                self.registers[X] = self.registers[X].wrapping_add(NN);
                // println!(" ---- result: {}", self.registers[X]);
            }	

            // 0x8XY0: Set register X to value of register Y
            (0x8, _, _, 0x0) => {
                // println!("ENTER - 8XY0");
                let b = self.registers[Y];
                self.registers[X] = b;
            }

            // 0x8XY1: Set register X to OR of registers X and Y
            (0x8, _, _, 0x1) => {
                // println!("ENTER - 8XY1");
                self.registers[X] = self.registers[X] | self.registers[Y];
            }

            // 0x8XY2: Set register X to AND of registers X and Y
            (0x8, _, _, 0x2) => {
                // print!("ENTER - 8XY2 - bitwise AND of reg:{}[{}] and reg:{}[{}]", X, self.registers[X], Y, self.registers[Y]);
                self.registers[X] = self.registers[X] & self.registers[Y];
                // println!(" ---- result is {}", self.registers[X]);
            }

            // 0x8XY3: Set register X to XOR of registers X and Y
            (0x8, _, _, 0x3) => {
                // println!("ENTER - 8XY3");
                self.registers[X] = self.registers[X] ^ self.registers[Y];
            }

            // 0x8XY4: Set register X to X + Y, set register F (15) to 1 or 0 depending on overflow
            (0x8, _, _, 0x4) => {
                // println!("ENTER - 8XY4");
                let vx = self.registers[X];
                let result = vx.wrapping_add(self.registers[Y]);
                self.registers[X] = result;
                if result < vx { // if result is less than original, we've had an overflow
                    self.registers[0xF] = 0x1;
                } else {
                    self.registers[0xF] = 0x0;
                }
            }

            // 0x8XY5: Subtract: set register X to the result of registers X - Y.
            (0x8, _,_, 0x5) => {
                // println!("ENTER - 8XY5");
                let wraps = self.registers[X] > self.registers[Y];
                self.registers[0xF] = if wraps { 0x1 } else { 0x0 };
        
                let result = self.registers[X].wrapping_sub(self.registers[Y]);
                self.registers[X] = result;
            }

            // 0x8XY6: Shift register X one step to the right after setting X to value of Y
            (0x8, _, _, 0x6) => {
                // println!("ENTER - 8XY6 - shift val in register {} after setting value from register {} [{}], new value: {}", X, Y, self.registers[Y], self.registers[Y] >> 1);
          
                // check if rightmost bit is set (and shifted out)
                self.registers[0xF] = if (self.registers[X]&(1<<0)) > 0 { 0x1 } else {0x0};
                
                self.registers[X] = self.registers[Y] >> 1;
            }

            // 0x8XY7: Subtract: set register X to the result of registers Y - X.
            (0x8, _, _, 0x7) => {
                // println!("ENTER - 8XY7");
                let notWrapping = self.registers[Y] > self.registers[X];
                self.registers[0xF] = if notWrapping { 0x1 } else { 0x0 };

                let result = self.registers[Y].wrapping_sub(self.registers[X]);
                self.registers[X] = result;
            }

            // 0x8XYE: Shift register X one step to the left
            (0x8, _, _, 0xE) => {
                // println!("ENTER - 8XYE");
                self.registers[X] = self.registers[Y];
                self.registers[0xF] = if (self.registers[X]&(1<<7)) > 0 { 0x1 } else {0x0};
                self.registers[X] = self.registers[X] << 1;
            }

            // 0x9: Skip if values in registers X and Y are not equal
            (0x9, _, _, 0x0) => {
                // println!("ENTER - 9XYN - skip instruction if reg {}[{}] != reg {}[{}]", X, self.registers[X], Y, self.registers[Y]);
                if self.registers[X] != self.registers[Y] {
                    self.pc += 2
                }
            }

            // 0xA: Set Index register to NNN
            (0xA, _, _, _) => {
                self.I = NNN;
            }

            // 0xB: Set PC to NNN + value in register 0
            (0xB, _, _, _) => {
                // original behaviour, assume register 0x0.
                self.pc = NNN; // + self.registers[0x0] as u16 
            }

            // 0xC: Random number into register X anded by NN
            (0xC, _, _, _) => {
                // print!("ENTER - CXNN - random in register {} anded by {}", X, NN);
                self.registers[X] = rand::random::<u8>() & NN;
                // println!(" ---- the result is {}",  self.registers[X]);
            }
        
            // 0xD: Draw
            (0xD, _, _, _) => {
                
                let xCoord = self.registers[X] % 64;
                let yCoord = self.registers[Y] % 32;
                
                self.registers[0xF] = 0x0;
                let mut firstByteIndex = self.I;
                
                for line in 0..N {
                    let spriteByte = self.memory[firstByteIndex as usize];
                    let row: u16 = (yCoord + line).into();

                    for bit in 0..8 {
                        let col: u16 = (xCoord + bit).into();
                        if spriteByte&(1<<(7-bit)) > 0 {

                            let index = (row*64+col)*4;
                            let mut handle = pixels.lock().unwrap();
                            let px = handle.get_frame_mut();
                            let isSet = px[index as usize] == 0xFF;
                            if isSet {
                                px[index as usize] = 0x0;
                                px[index as usize +1] = 0x0;
                                px[index as usize +2] = 0x0;
                                px[index as usize+ 3] = 0xff;
                                self.registers[0xF] = 0x1
                            } else {
                                px[index as usize] = 0xFF;
                                px[index as usize +1] = 0xFF;
                                px[index as usize +2] = 0xFF;
                                px[index as usize+ 3] = 0xff;
                            }
                        }
                    }
                    firstByteIndex+=1;
                }
            }

            // EX9E: handle key pressed
            (0xE, _, 0x9, 0xE) => {
                let keyPressed = input.lock().unwrap().key_held(keyCode(self.registers[X]));
                if keyPressed {
                    self.pc += 2;
                }
            }
            // EXA1: handle key not pressed
            (0xE, _, 0xA, 0x1) => {
                let keyPressed = input.lock().unwrap().key_held(keyCode(self.registers[X]));
                if !keyPressed {
                    self.pc += 2;
                }
            }

            // 0xFX07 -  Set register X to current value of delay timer
            (0xF, _, 0x0, 0x7) => {
                // println!("ENTER FX07 - set value of delay timer [{}] to register {}", self.delayTimer, X);
                self.registers[X] = self.delayTimer;
            }
            // 0xFX15 -  Set the delay timer to value of register X
            (0xF, _, 0x1, 0x5) => {
                // println!("ENTER FX15 - set delay timer to value of register {}[{}]", X, self.registers[X]);
                self.delayTimer = self.registers[X];
            }
            // 0xFX18 -  Set the sound timer to value of register X
            (0xF, _, 0x1, 0x8) => {
                self.soundTimer = self.registers[X];
            }

            // 0xFX1E - Add to index: Add value of register X to I
            (0xF, _, 0x1, 0xE) => {
                let mut i = self.I + self.registers[X] as u16;
                // old-school amiga behaviour
                if i > 0xFFF { 
                    self.registers[0xF] = 0x1;
                    i = i % 0x1000; //  mod 4096 in case of overflow over original 4kb of RAM
                } else {
                    self.registers[0xF] = 0x0;
                }
               self.I = i;
            }

            // 0xFX0A: Get key (blocks until input is received)
            (0xF, _, 0x0, 0xA) => {
                // TODO!
            }

             // 0xFX29: font character, sets I to first byte of character per register X
             (0xF, _, 0x2, 0x9) => {
                let b = self.registers[X] & 0x0F; // just use last nibble of value in register X
				self.I = FONT_OFFSET + (b*5) as u16; //fontOffsets[b];
            }

            // 0xFX33: binary-coded decimal conversion. Note that "10" is split into 0,1,0 and 4 into 0,0,4.
            (0xF, _, 0x3, 0x3) => {
                // println!("ENTER - FX33");
                self.memory[self.I as usize] = (self.registers[X] / 100) % 10;
				self.memory[(self.I+1) as usize] = (self.registers[X] / 10) % 10;
				self.memory[(self.I+2) as usize] = (self.registers[X] / 1) % 10;
            }

            // 0xFX55: Store register to memory
            (0xF, _, 0x5, 0x5) => {
                // println!("ENTER - FX55");
                let to = X+1;
                for i in 0..to {
					let index = self.I + i as u16;
					self.memory[index as usize] = self.registers[i];
                    // println!("   -> stored reg {}[{}] in memory index {}", i, self.registers[i], index)
				}
                if FORCE_COSMAC_VIP {
                    self.I = self.I + (X+1) as u16;
                }
            }
            // 0xFX65: Load value from memory into register
            (0xF, _, 0x6, 0x5) => {
                // println!("ENTER - FX65");
                let to = X+1;
                for i in 0..to {
                    let index = (self.I + i as u16) as u16;
					self.registers[i] = self.memory[index as usize];
                    // println!("   -> loaded value from memory index {}[{}] into register {}", self.I, self.memory[index as usize], i);
					
                    if FORCE_COSMAC_VIP {
                        self.I = self.I + 1;
                    }
				}
            }

            // print any missing instructions.
            (_instr, _X, _Y, _N) =>  println!("catch all!"),
        }
    
    }
}

fn keyCode(x: u8) -> VirtualKeyCode {

        match x {
        0x0  => VirtualKeyCode::Key0,
        0x1 => VirtualKeyCode::Key1,
        0x2  => VirtualKeyCode::Key2,
        0x3  => VirtualKeyCode::Key3,
        0x4  => VirtualKeyCode::Key4,
        0x5  => VirtualKeyCode::Key5,
        0x6  => VirtualKeyCode::Key6,
        0x7  => VirtualKeyCode::Key7,
        0x8  => VirtualKeyCode::Key8,
        0x9  => VirtualKeyCode::Key9,
        0xA  => VirtualKeyCode::A,
        0xB  => VirtualKeyCode::B,
        0xC  => VirtualKeyCode::C,
        0xD  => VirtualKeyCode::D,
        0xE  => VirtualKeyCode::E,
        0xF  => VirtualKeyCode::F,
        _ => VirtualKeyCode::Escape,
    }
}

static FONT: [u8; 80] = [
    0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
    0x20, 0x60, 0x20, 0x20, 0x70, // 1
    0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
    0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
    0x90, 0x90, 0xF0, 0x10, 0x10, // 4
    0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
    0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
    0xF0, 0x10, 0x20, 0x40, 0x40, // 7
    0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
    0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
    0xF0, 0x90, 0xF0, 0x90, 0x90, // A
    0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
    0xF0, 0x80, 0x80, 0x80, 0xF0, // C
    0xE0, 0x90, 0x90, 0x90, 0xE0, // D
    0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
    0xF0, 0x80, 0xF0, 0x80, 0x80, // F
];
