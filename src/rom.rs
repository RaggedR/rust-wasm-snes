/// SNES ROM / Cartridge loader.
///
/// Handles .smc files (with optional 512-byte copier header), detects
/// LoROM vs HiROM mapping by scoring header candidates at both offsets,
/// and exposes the raw ROM data for memory mapping.

use std::fmt;
use std::fs;
use std::path::Path;

const COPIER_HEADER_SIZE: usize = 512;
const LOROM_HEADER_OFFSET: usize = 0x7FC0;
const HIROM_HEADER_OFFSET: usize = 0xFFC0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMode {
    LoROM,
    HiROM,
}

pub struct Cartridge {
    /// Raw ROM data with copier header stripped.
    pub rom: Vec<u8>,
    /// Battery-backed SRAM (8KB for LTTP).
    pub sram: Vec<u8>,
    pub title: String,
    pub map_mode: MapMode,
    pub rom_size: usize,
    pub ram_size: usize,
    pub country: u8,
    pub version: u8,
    pub checksum: u16,
    pub checksum_complement: u16,
}

/// Score a candidate header location. Higher score = more likely to be
/// the real header. Checks checksum validity, map-mode byte consistency,
/// and title character plausibility.
fn score_header(rom: &[u8], offset: usize, expect_hirom: bool) -> u32 {
    if offset + 64 > rom.len() { return 0; }
    let h = &rom[offset..];
    let mut score = 0u32;

    // Checksum + complement should equal $FFFF.
    let complement = u16::from_le_bytes([h[0x1C], h[0x1D]]);
    let checksum = u16::from_le_bytes([h[0x1E], h[0x1F]]);
    if checksum.wrapping_add(complement) == 0xFFFF {
        score += 4;
    }

    // Map-mode byte's bit 0 should agree with the expected mapping.
    let map_byte = h[0x15];
    let is_hirom = map_byte & 0x01 != 0;
    if is_hirom == expect_hirom {
        score += 2;
    }

    // Title bytes should be printable ASCII (space through tilde).
    for &b in &h[0..21] {
        if (0x20..=0x7E).contains(&b) {
            score += 1;
        }
    }

    score
}

/// Detect whether a ROM image is LoROM or HiROM by scoring headers at
/// both candidate offsets ($7FC0 for LoROM, $FFC0 for HiROM).
pub fn detect_map_mode(rom: &[u8]) -> MapMode {
    let lo_score = score_header(rom, LOROM_HEADER_OFFSET, false);
    let hi_score = score_header(rom, HIROM_HEADER_OFFSET, true);
    if hi_score > lo_score { MapMode::HiROM } else { MapMode::LoROM }
}

/// Return the byte offset of the internal header for the given map mode.
pub fn header_offset(mode: MapMode) -> usize {
    match mode {
        MapMode::LoROM => LOROM_HEADER_OFFSET,
        MapMode::HiROM => HIROM_HEADER_OFFSET,
    }
}

/// Decode the ROM type byte ($xFD6) into a human-readable chip name.
/// Returns `None` for plain ROM/RAM/battery configurations that don't
/// require a coprocessor.
pub fn special_chip_name(map_byte: u8, rom_type: u8) -> Option<&'static str> {
    // Map byte high nibble encodes extended mappings.
    let base_map = map_byte & 0x0F;
    match base_map {
        0x03 => return Some("SA-1"),
        0x05 => return Some("ExHiROM"),
        _ => {}
    }

    // ROM type byte: values $03+ indicate a coprocessor.
    // The coprocessor identity is encoded in bits 4-7 of the map byte
    // combined with the ROM type. Simplified lookup for common chips:
    match rom_type {
        0x00..=0x02 => None, // ROM only, ROM+RAM, ROM+RAM+Battery
        0x03..=0x06 => {
            // Coprocessor present — identify by map byte high nibble.
            match map_byte & 0xF0 {
                0x00 | 0x30 => Some("DSP-1"),
                0x20 => Some("OBC-1"),
                _ => Some("unknown coprocessor"),
            }
        }
        0x13..=0x16 => Some("SuperFX"),
        0x25 | 0x35 => Some("S-RTC"),
        0x33..=0x36 => Some("SA-1"),
        0x43..=0x46 => Some("S-DD1"),
        0x53..=0x56 => Some("S-RTC"),
        0xE3..=0xE6 => Some("other (Game Boy, etc.)"),
        0xF3..=0xF6 => Some("ST010/ST011"),
        _ => Some("unknown coprocessor"),
    }
}

impl Cartridge {
    pub fn load(path: &Path) -> Result<Self, String> {
        let data = fs::read(path).map_err(|e| format!("Failed to read ROM: {e}"))?;

        // Detect and strip copier header.
        // If file size mod 1024 == 512, there's a 512-byte copier header.
        let rom = if data.len() % 1024 == COPIER_HEADER_SIZE {
            println!(
                "Detected {COPIER_HEADER_SIZE}-byte copier header, stripping..."
            );
            data[COPIER_HEADER_SIZE..].to_vec()
        } else {
            data
        };

        if rom.len() < LOROM_HEADER_OFFSET + 64 {
            return Err(format!(
                "ROM too small ({} bytes) to contain internal header",
                rom.len()
            ));
        }

        // Detect mapping mode by scoring both header locations.
        let map_mode = detect_map_mode(&rom);
        let h_off = header_offset(map_mode);
        let h = &rom[h_off..];

        let title = String::from_utf8_lossy(&h[0..21]).trim().to_string();

        let map_byte = h[0x15];
        let rom_type = h[0x16]; // $xFD6

        let rom_size_code = h[0x17]; // $xFD7
        let rom_size = 1024 << rom_size_code; // 2^N KB

        let ram_size_code = h[0x18]; // $xFD8
        let ram_size = if ram_size_code == 0 {
            0
        } else {
            1024 << ram_size_code
        };

        let country = h[0x19]; // $xFD9
        let version = h[0x1B]; // $xFDB

        let checksum_complement = u16::from_le_bytes([h[0x1C], h[0x1D]]); // $xFDC
        let checksum = u16::from_le_bytes([h[0x1E], h[0x1F]]); // $xFDE

        // Warn about special chips that aren't emulated.
        if let Some(chip) = special_chip_name(map_byte, rom_type) {
            eprintln!(
                "WARNING: This ROM requires {} which is not emulated. \
                 Expect incorrect behavior.",
                chip
            );
        }

        // SRAM — try loading from .srm file alongside the ROM.
        let srm_path = path.with_extension("srm");
        let sram = if srm_path.exists() {
            let data = std::fs::read(&srm_path)
                .unwrap_or_else(|_| vec![0u8; ram_size]);
            eprintln!("Loaded SRAM from {}", srm_path.display());
            let mut s = data;
            s.resize(ram_size, 0);
            s
        } else {
            vec![0u8; ram_size]
        };

        let cart = Self {
            rom,
            sram,
            title,
            map_mode,
            rom_size,
            ram_size,
            country,
            version,
            checksum,
            checksum_complement,
        };

        println!("{cart}");

        // Verify checksum complement.
        if checksum.wrapping_add(checksum_complement) != 0xFFFF {
            println!(
                "WARNING: checksum + complement = {:#06X} (expected 0xFFFF)",
                checksum.wrapping_add(checksum_complement)
            );
        }

        Ok(cart)
    }

    /// Read a byte from ROM. The offset formula depends on the map mode:
    ///
    /// * **LoROM**: each bank contributes 32 KB at $8000-$FFFF.
    ///   `offset = (bank & $7F) × $8000 + (addr − $8000)`
    ///
    /// * **HiROM**: each bank contributes a full 64 KB, wrapping within
    ///   the bottom 6 bits of the bank byte.
    ///   `offset = (bank & $3F) × $10000 + addr`
    ///
    /// Out-of-range offsets wrap around the ROM size (mirroring), which
    /// matches real hardware behaviour for under-decoded address lines.
    pub fn read(&self, bank: u8, addr: u16) -> u8 {
        let offset = match self.map_mode {
            MapMode::LoROM => {
                ((bank & 0x7F) as usize) * 0x8000
                    + (addr as usize).wrapping_sub(0x8000)
            }
            MapMode::HiROM => {
                ((bank & 0x3F) as usize) * 0x10000 + addr as usize
            }
        };
        if self.rom.is_empty() {
            return 0;
        }
        self.rom[offset % self.rom.len()]
    }
}

impl fmt::Display for Cartridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ROM: \"{}\" | {:?} | {}KB ROM | {}KB SRAM | v{} | checksum: {:#06X}",
            self.title,
            self.map_mode,
            self.rom_size / 1024,
            self.ram_size / 1024,
            self.version,
            self.checksum,
        )
    }
}
