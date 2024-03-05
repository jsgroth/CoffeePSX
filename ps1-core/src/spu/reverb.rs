#[derive(Debug, Clone, Copy, Default)]
struct StereoValue<T> {
    l: T,
    r: T,
}

type StereoI32 = StereoValue<i32>;
type StereoU32 = StereoValue<u32>;

#[derive(Debug, Clone, Default)]
pub struct ReverbSettings {
    pub writes_enabled: bool,
    buffer_start_addr: u32,
    buffer_current_addr: u32,
    input_volume: StereoI32,
    output_volume: StereoI32,
    comb_volumes: [i32; 4],
    reflection_volume_1: i32,
    reflection_volume_2: i32,
    apf_volume_1: i32,
    apf_volume_2: i32,
    comb_addrs: [StereoU32; 4],
    same_reflect_addr_1: StereoU32,
    same_reflect_addr_2: StereoU32,
    diff_reflect_addr_1: StereoU32,
    diff_reflect_addr_2: StereoU32,
    apf_addr_1: StereoU32,
    apf_offset_1: u32,
    apf_addr_2: StereoU32,
    apf_offset_2: u32,
}

impl ReverbSettings {
    // $1F801D84: Reverb output volume L (vLOUT)
    pub fn write_output_volume_l(&mut self, value: u32) {
        self.output_volume.l = parse_volume(value);
        log::trace!("Reverb output volume L: {}", self.output_volume.l);
    }

    // $1F801D86: Reverb output volume R (vROUT)
    pub fn write_output_volume_r(&mut self, value: u32) {
        self.output_volume.r = parse_volume(value);
        log::trace!("Reverb output volume R: {}", self.output_volume.r);
    }

    // $1F801DA2: Reverb buffer start address (mBASE)
    pub fn write_buffer_start_address(&mut self, value: u32) {
        // Writing start address also sets current address
        self.buffer_start_addr = parse_address(value);
        self.buffer_current_addr = self.buffer_start_addr;
        log::trace!(
            "Reverb buffer start address: {:05X}",
            self.buffer_start_addr
        );
    }

    // $1F801DC0-$1F801DFF: The majority of the reverb registers
    pub fn write_register(&mut self, address: u32, value: u32) {
        match address & 0xFFFF {
            0x1DC0 => self.write_dapf1(value),
            0x1DC2 => self.write_dapf2(value),
            0x1DC4 => self.write_viir(value),
            0x1DC6 => self.write_vcomb1(value),
            0x1DC8 => self.write_vcomb2(value),
            0x1DCA => self.write_vcomb3(value),
            0x1DCC => self.write_vcomb4(value),
            0x1DCE => self.write_vwall(value),
            0x1DD0 => self.write_vapf1(value),
            0x1DD2 => self.write_vapf2(value),
            0x1DD4 => self.write_mlsame(value),
            0x1DD6 => self.write_mrsame(value),
            0x1DD8 => self.write_mlcomb1(value),
            0x1DDA => self.write_mrcomb1(value),
            0x1DDC => self.write_mlcomb2(value),
            0x1DDE => self.write_mrcomb2(value),
            0x1DE0 => self.write_dlsame(value),
            0x1DE2 => self.write_drsame(value),
            0x1DE4 => self.write_mldiff(value),
            0x1DE6 => self.write_mrdiff(value),
            0x1DE8 => self.write_mlcomb3(value),
            0x1DEA => self.write_mrcomb3(value),
            0x1DEC => self.write_mlcomb4(value),
            0x1DEE => self.write_mrcomb4(value),
            0x1DF0 => self.write_dldiff(value),
            0x1DF2 => self.write_drdiff(value),
            0x1DF4 => self.write_mlapf1(value),
            0x1DF6 => self.write_mrapf1(value),
            0x1DF8 => self.write_mlapf2(value),
            0x1DFA => self.write_mrapf2(value),
            0x1DFC => self.write_vlin(value),
            0x1DFE => self.write_vrin(value),
            _ => todo!("reverb register write {address:08X} {value:04X}"),
        }
    }

    // $1F801DC0: APF offset 1 (dAPF1)
    fn write_dapf1(&mut self, value: u32) {
        self.apf_offset_1 = parse_address(value);
        log::trace!("dAPF1: {:05X}", self.apf_offset_1);
    }

    // $1F801DC2: APF offset 2 (dAPF2)
    fn write_dapf2(&mut self, value: u32) {
        self.apf_offset_2 = parse_address(value);
        log::trace!("dAPF2: {:05X}", self.apf_offset_2);
    }

    // $1F801DC4: Reflection volume 1 (vIIR)
    fn write_viir(&mut self, value: u32) {
        self.reflection_volume_1 = parse_volume(value);
        log::trace!("vIIR: {}", self.reflection_volume_1);
    }

    // $1F801DC6: Comb volume 1 (vCOMB1)
    fn write_vcomb1(&mut self, value: u32) {
        self.comb_volumes[0] = parse_volume(value);
        log::trace!("vCOMB1: {}", self.comb_volumes[0]);
    }

    // $1F801DC8: Comb volume 2 (vCOMB2)
    fn write_vcomb2(&mut self, value: u32) {
        self.comb_volumes[1] = parse_volume(value);
        log::trace!("vCOMB2: {}", self.comb_volumes[1]);
    }

    // $1F801DCA: Comb volume 3 (vCOMB3)
    fn write_vcomb3(&mut self, value: u32) {
        self.comb_volumes[2] = parse_volume(value);
        log::trace!("vCOMB3: {}", self.comb_volumes[2]);
    }

    // $1F801DCC: Comb volume 4 (vCOMB4)
    fn write_vcomb4(&mut self, value: u32) {
        self.comb_volumes[3] = parse_volume(value);
        log::trace!("vCOMB4: {}", self.comb_volumes[3]);
    }

    // $1F801DCE: Reflection volume 2 (vWALL)
    fn write_vwall(&mut self, value: u32) {
        self.reflection_volume_2 = parse_volume(value);
        log::trace!("vWALL: {}", self.reflection_volume_2);
    }

    // $1F801DD0: APF volume 1 (vAPF1)
    fn write_vapf1(&mut self, value: u32) {
        self.apf_volume_1 = parse_volume(value);
        log::trace!("vAPF1: {}", self.apf_volume_1);
    }

    // $1F801DD2: APF volume 2 (vAPF2)
    fn write_vapf2(&mut self, value: u32) {
        self.apf_volume_2 = parse_volume(value);
        log::trace!("vAPF2: {}", self.apf_volume_2);
    }

    // $1F801DD4: Same-side reflection address 1 left (mLSAME)
    fn write_mlsame(&mut self, value: u32) {
        self.same_reflect_addr_1.l = parse_address(value);
        log::trace!("mLSAME: {:05X}", self.same_reflect_addr_1.l);
    }

    // $1F801DD6: Same-side reflection address 1 right (mRSAME)
    fn write_mrsame(&mut self, value: u32) {
        self.same_reflect_addr_1.r = parse_address(value);
        log::trace!("mRSAME: {:05X}", self.same_reflect_addr_1.r);
    }

    // $1F801DD8: Comb address 1 left (mLCOMB1)
    fn write_mlcomb1(&mut self, value: u32) {
        self.comb_addrs[0].l = parse_address(value);
        log::trace!("mLCOMB1: {:05X}", self.comb_addrs[0].l);
    }

    // $1F801DDA: Comb address 1 right (mRCOMB1)
    fn write_mrcomb1(&mut self, value: u32) {
        self.comb_addrs[0].r = parse_address(value);
        log::trace!("mRCOMB1: {:05X}", self.comb_addrs[0].r);
    }

    // $1F801DDC: Comb address 2 left (mLCOMB2)
    fn write_mlcomb2(&mut self, value: u32) {
        self.comb_addrs[1].l = parse_address(value);
        log::trace!("mLCOMB2: {:05X}", self.comb_addrs[1].l);
    }

    // $1F801DDE: Comb address 2 right (mRCOMB2)
    fn write_mrcomb2(&mut self, value: u32) {
        self.comb_addrs[1].r = parse_address(value);
        log::trace!("mRCOMB2: {:05X}", self.comb_addrs[1].r);
    }

    // $1F801DE0: Same-side reflection address 2 left (dLSAME)
    fn write_dlsame(&mut self, value: u32) {
        self.same_reflect_addr_2.l = parse_address(value);
        log::trace!("dLSAME: {:05X}", self.same_reflect_addr_2.l);
    }

    // $1F801DE2: Same-side reflection address 2 right (dRSAME)
    fn write_drsame(&mut self, value: u32) {
        self.same_reflect_addr_2.r = parse_address(value);
        log::trace!("dRSAME: {:05X}", self.same_reflect_addr_2.r);
    }

    // $1F801DE4: Different-side reflection address 1 left (mLDIFF)
    fn write_mldiff(&mut self, value: u32) {
        self.diff_reflect_addr_1.l = parse_address(value);
        log::trace!("mLDIFF: {:05X}", self.diff_reflect_addr_1.l);
    }

    // $1F801DE6: Different-side reflection address 1 right (mRDIFF)
    fn write_mrdiff(&mut self, value: u32) {
        self.diff_reflect_addr_1.r = parse_address(value);
        log::trace!("mRDIFF: {:05X}", self.diff_reflect_addr_1.r);
    }

    // $1F801DE8: Comb address 3 left (mLCOMB3)
    fn write_mlcomb3(&mut self, value: u32) {
        self.comb_addrs[2].l = parse_address(value);
        log::trace!("mLCOMB3: {:05X}", self.comb_addrs[2].l);
    }

    // $1F801DEA: Comb address 3 right (mRCOMB3)
    fn write_mrcomb3(&mut self, value: u32) {
        self.comb_addrs[2].r = parse_address(value);
        log::trace!("mRCOMB3: {:05X}", self.comb_addrs[2].r);
    }

    // $1F801DEC: Comb address 4 left (mLCOMB4)
    fn write_mlcomb4(&mut self, value: u32) {
        self.comb_addrs[3].l = parse_address(value);
        log::trace!("mLCOMB4: {:05X}", self.comb_addrs[3].l);
    }

    // $1F801DEE: Comb address 4 right (mRCOMB4)
    fn write_mrcomb4(&mut self, value: u32) {
        self.comb_addrs[3].r = parse_address(value);
        log::trace!("mRCOMB4: {:05X}", self.comb_addrs[3].r);
    }

    // $1F801DF0: Different-side reflection address 2 left (dLDIFF)
    fn write_dldiff(&mut self, value: u32) {
        self.diff_reflect_addr_2.l = parse_address(value);
        log::trace!("dLDIFF: {:05X}", self.diff_reflect_addr_2.l);
    }

    // $1F801DF2: Different-side reflection address 2 right (dRDIFF)
    fn write_drdiff(&mut self, value: u32) {
        self.diff_reflect_addr_2.r = parse_address(value);
        log::trace!("dRDIFF: {:05X}", self.diff_reflect_addr_2.r);
    }

    // $1F801DF4: APF address 1 left (mLAPF1)
    fn write_mlapf1(&mut self, value: u32) {
        self.apf_addr_1.l = parse_address(value);
        log::trace!("mLAPF1: {:05X}", self.apf_addr_1.l);
    }

    // $1F801DF6: APF address 1 right (mRAPF1)
    fn write_mrapf1(&mut self, value: u32) {
        self.apf_addr_1.r = parse_address(value);
        log::trace!("mRAPF1: {:05X}", self.apf_addr_1.r);
    }

    // $1F801DF8: APF address 2 left (mLAPF2)
    fn write_mlapf2(&mut self, value: u32) {
        self.apf_addr_2.l = parse_address(value);
        log::trace!("mLAPF2: {:05X}", self.apf_addr_2.l);
    }

    // $1F801DFA: APF address 2 right (mRAPF2)
    fn write_mrapf2(&mut self, value: u32) {
        self.apf_addr_2.r = parse_address(value);
        log::trace!("mRAPF2: {:05X}", self.apf_addr_2.r);
    }

    // $1F801DFC: Input volume left (vLIN)
    fn write_vlin(&mut self, value: u32) {
        self.input_volume.l = parse_volume(value);
        log::trace!("vLIN: {}", self.input_volume.l);
    }

    // $1F801DFE: Input volume right (vRIN)
    fn write_vrin(&mut self, value: u32) {
        self.input_volume.r = parse_volume(value);
        log::trace!("vRIN: {}", self.input_volume.r);
    }
}

fn parse_address(value: u32) -> u32 {
    // All address registers are in 8-byte units
    (value & 0xFFFF) << 3
}

fn parse_volume(value: u32) -> i32 {
    // All volume registers are signed 16-bit values
    (value as i16).into()
}
