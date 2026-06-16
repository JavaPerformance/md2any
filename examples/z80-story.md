---
title: The Z80 Story
subtitle: A Retro Computing Legend
theme: midnight
style:
  accent: #22D3EE
  bg: #0F172A
  title_color: #E0F2FE
aspect: 16:9
---

# The Z80 Story

## The Rebel Engineer
Federico Faggin arrived at Intel in 1970 already a veteran of Fairchild's breakthroughs. He poured himself into the 4004 and then the 8080, only to watch management grow cautious and short-sighted. By 1974 the frustration had become unbearable; Faggin walked out, determined to build something better on his own terms.

## Founding Zilog
<!-- layout: image-right -->
![Federico Faggin](assets/z80-story/faggin.jpg)

With Ralph Ungermann he launched Zilog that same year. The mission was simple and audacious: surpass Intel at its own game. They lured away the brightest engineers and set to work on a microprocessor that would keep every 8080 program running while offering far more power and elegance.

# The Launch

## The Gamble
In July 1976 the Z80 hit the market at a price that undercut the 8080 yet delivered capabilities Intel had never imagined. Hobbyists embraced it instantly; manufacturers lined up to second-source it. The gamble had paid off—the chip that shouldn't have existed was suddenly everywhere.

## Silicon Heart
<!-- layout: image-left -->
![Z80 chip package](assets/z80-story/z80_chip.jpg)

Inside the familiar 40-pin package lived an architecture that felt almost alive. Full binary compatibility meant existing 8080 software ran without change, while new instructions and registers opened doors the older chip had kept firmly shut.

## The Die Revealed
<!-- layout: image-full -->
![Z80 die shot](assets/z80-story/z80_die.jpg)

## What Made It Special
The Z80 carried alternate register banks that let programmers swap contexts in a single instruction. Fresh IX and IY index registers turned awkward memory access into clean, readable code. Best of all, it needed only a single +5 V rail—no more juggling three separate voltages just to keep the machine alive.

## Z80 vs Intel 8080
| Feature          | Z80                  | Intel 8080          |
|------------------|----------------------|---------------------|
| Registers        | 22 total (alt + IX/IY) | 8 main + few        |
| Clock speed      | up to 20 MHz         | up to 10 MHz        |
| Power supply     | single +5 V          | +5 / -5 / +12 V     |
| Instructions     | 158 (incl. block)    | 78                  |

## Built-in DRAM Refresh
Hidden inside the silicon was circuitry that refreshed dynamic RAM without any extra chips or timing headaches. Systems became smaller, cheaper, and more reliable overnight—an advantage that mattered most to the cost-conscious engineers building the first wave of personal computers.

# Software Ecosystem

## CP/M and the Software Boom
Gary Kildall's CP/M found its perfect partner in the Z80. The extra registers let the operating system run faster and smoother than on the 8080, and a flood of business tools, languages, and utilities followed. For the first time, ordinary people could buy software instead of writing every line themselves.

## Z80 Assembly Example
```asm
delay: LD   B, 0FFh   ; outer loop count
inner: LD   A, (HL)   ; load byte
       INC  HL
       DJNZ inner     ; dec B, loop if !=0
       RET
```

## Z80 Block Copy Example
```asm
; Copy BC bytes from (HL) to (DE)
memcpy: LD   A, (HL)   ; load source byte
        LD   (DE), A   ; store to destination
        INC  HL
        INC  DE
        DEC  BC
        LD   A, B
        OR   C
        JR   NZ, memcpy
        RET
```

## The Software Explosion
<!-- layout: text-full -->
Assemblers, BASIC interpreters, early spreadsheets, and bedroom-coded games poured forth. The Z80 turned every machine it powered into a platform, laying the foundation for the personal-computing software industry we still inhabit today.

# Home Computers

## The Machines That Defined an Era
<!-- layout: image-left -->
![ZX Spectrum](assets/z80-story/zx_spectrum.jpg)

From the moment the TRS-80 Model I appeared in 1977, the Z80 became the heartbeat of home computing. Sinclair's ZX Spectrum brought vivid color and bedroom coding to millions; the Amstrad CPC and Japanese MSX standard carried the same spirit across continents.

## Iconic Z80 Machines
| Machine            | Launch |
|--------------------|--------|
| TRS-80 Model I     | 1977   |
| ZX Spectrum        | 1982   |
| MSX                | 1983   |
| Amstrad CPC        | 1984   |
| Sega Master System | 1985   |
| Nintendo Game Boy  | 1989   |

# Gaming Consoles

## Sega Master System
Released in 1985, the Master System ran its Z80 at 3.58 MHz alongside a superb sound chip. Titles like Phantasy Star proved the little processor could still deliver rich worlds long after its desktop rivals had moved on.

## Nintendo Game Boy
<!-- layout: image-left valign=center -->
![Game Boy](assets/z80-story/gameboy.jpg)

The original Game Boy (1989) used Sharp's LR35902, a Z80-derived CPU running at ~4.19 MHz with a reduced instruction set. Its efficient design, combined with clever graphics hardware, launched the portable gaming era and sold more than 118 million units worldwide.

## Game Boy's Sharp LR35902
Nintendo's 1989 handheld used a custom Z80 variant stripped for efficiency yet still unmistakably Z80 at heart. Paired with clever graphics hardware, it launched the portable-gaming revolution and sold more than 118 million units.

Sharp designed the LR35902 in 1989 under contract from Nintendo. It inherits the Z80’s core architecture and instruction set but removes alternate register banks, IX/IY index registers, and most block-transfer instructions to reduce silicon area and power. The chip runs at 4.194304 MHz, uses a unified 8-bit data bus, and adds a handful of new opcodes for direct bit manipulation and faster LCD access. DRAM-refresh circuitry was omitted because the Game Boy employs only static RAM.

# Industrial Longevity

## The Chip That Wouldn't Die
While flashier processors came and went, the Z80 remained in production for over forty-five years. Its rock-solid reliability made it the default choice for traffic lights, medical instruments, printers, and factory controllers—quietly running the modern world long after its glory days in home computers.

## Modern Legacy
<!-- layout: image-left -->
![Modern Z80 (Z84C00)](https://upload.wikimedia.org/wikipedia/commons/4/40/Z84C0010FEC_LQFP.png)

Open-source cores such as T80 keep the Z80 alive inside FPGAs. Retro enthusiasts and chiptune artists still celebrate its elegant instruction set. More than a processor, it became a symbol of practical, enduring engineering—the rebel chip that refused to fade away.

*Photo: HenkeB / Public domain*

## Z80 Today
Zilog still ships new Z80 parts in 2024. You’ll find them in smart electricity meters, guitar effects pedals, industrial controllers, and retro-computing kits. Open-source cores (T80, Z80SoftCore) keep the architecture alive inside FPGAs, while hobbyists continue writing new software for classic machines.

## Z80 Behind the Iron Curtain
Eastern Bloc factories produced near-perfect clones for decades: the Soviet T34VM1, East German U880, Romanian MMN80, and Bulgarian clone all powered locally-made computers and industrial gear. Because the Z80 needed only a single 5 V supply and ran CP/M, it became the unofficial standard across the Warsaw Pact.

## Z80 in the Arcades
The same chip that powered home computers also dominated early coin-op machines. Pac-Man, Galaga, Frogger, and dozens of 1980–83 titles ran on Z80-based boards from Namco, Sega, and Konami. Its rich instruction set and low cost made it the go-to CPU until 16-bit systems arrived.

## Image Credits

Photographs via Wikimedia Commons, reused under their Creative Commons licenses:

- Z80 die shot — ZeptoBars (CC BY 3.0)
- Z80 chip package — Gennadiy Shvets (CC BY 2.5)
- Federico Faggin — Intel Free Press (CC BY-SA 2.0)
- ZX Spectrum — Bill Bertram / Pixel8 (CC BY-SA 2.5)