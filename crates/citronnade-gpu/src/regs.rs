//! PICA200 register numbers (as written in command lists), with the names
//! homebrew documentation uses.
#![allow(missing_docs)]
/// Used to finalize GPU drawing.
pub const FINALIZE: u16 = 0x0010;
/// Face culling configuration.
pub const FACECULLING_CONFIG: u16 = 0x0040;
/// Viewport width.
pub const VIEWPORT_WIDTH: u16 = 0x0041;
/// Inverted viewport width.
pub const VIEWPORT_INVW: u16 = 0x0042;
/// Viewport height.
pub const VIEWPORT_HEIGHT: u16 = 0x0043;
/// Inverted viewport height.
pub const VIEWPORT_INVH: u16 = 0x0044;
/// Unknown
pub const FRAGOP_CLIP: u16 = 0x0047;
/// Unknown
pub const FRAGOP_CLIP_DATA0: u16 = 0x0048;
/// Unknown
pub const FRAGOP_CLIP_DATA1: u16 = 0x0049;
/// Unknown
pub const FRAGOP_CLIP_DATA2: u16 = 0x004A;
/// Unknown
pub const FRAGOP_CLIP_DATA3: u16 = 0x004B;
/// Depth map scale.
pub const DEPTHMAP_SCALE: u16 = 0x004D;
/// Depth map offset.
pub const DEPTHMAP_OFFSET: u16 = 0x004E;
/// Shader output map total.
pub const SH_OUTMAP_TOTAL: u16 = 0x004F;
/// Shader output map 0.
pub const SH_OUTMAP_O0: u16 = 0x0050;
/// Shader output map 1.
pub const SH_OUTMAP_O1: u16 = 0x0051;
/// Shader output map 2.
pub const SH_OUTMAP_O2: u16 = 0x0052;
/// Shader output map 3.
pub const SH_OUTMAP_O3: u16 = 0x0053;
/// Shader output map 4.
pub const SH_OUTMAP_O4: u16 = 0x0054;
/// Shader output map 5.
pub const SH_OUTMAP_O5: u16 = 0x0055;
/// Shader output map 6.
pub const SH_OUTMAP_O6: u16 = 0x0056;
/// Unknown
pub const EARLYDEPTH_FUNC: u16 = 0x0061;
/// Unknown
pub const EARLYDEPTH_TEST1: u16 = 0x0062;
/// Unknown
pub const EARLYDEPTH_CLEAR: u16 = 0x0063;
/// Shader output attributes mode.
pub const SH_OUTATTR_MODE: u16 = 0x0064;
/// Scissor test mode.
pub const SCISSORTEST_MODE: u16 = 0x0065;
/// Scissor test position.
pub const SCISSORTEST_POS: u16 = 0x0066;
/// Scissor text dimensions.
pub const SCISSORTEST_DIM: u16 = 0x0067;
/// Viewport X and Y.
pub const VIEWPORT_XY: u16 = 0x0068;
/// Unknown
pub const EARLYDEPTH_DATA: u16 = 0x006A;
/// Depth map enable.
pub const DEPTHMAP_ENABLE: u16 = 0x006D;
/// Renderbuffer dimensions.
pub const RENDERBUF_DIM: u16 = 0x006E;
/// Shader output attributes clock enable.
pub const SH_OUTATTR_CLOCK: u16 = 0x006F;
/// Texture unit configuration.
pub const TEXUNIT_CONFIG: u16 = 0x0080;
/// Texture unit 0 border color.
pub const TEXUNIT0_BORDER_COLOR: u16 = 0x0081;
/// Texture unit 0 dimensions.
pub const TEXUNIT0_DIM: u16 = 0x0082;
/// Texture unit 0 parameters.
pub const TEXUNIT0_PARAM: u16 = 0x0083;
/// Texture unit 0 LOD.
pub const TEXUNIT0_LOD: u16 = 0x0084;
/// Texture unit 0 address.
pub const TEXUNIT0_ADDR1: u16 = 0x0085;
pub const TEXUNIT0_ADDR2: u16 = 0x0086;
pub const TEXUNIT0_ADDR3: u16 = 0x0087;
pub const TEXUNIT0_ADDR4: u16 = 0x0088;
pub const TEXUNIT0_ADDR5: u16 = 0x0089;
pub const TEXUNIT0_ADDR6: u16 = 0x008A;
pub const TEXUNIT0_SHADOW: u16 = 0x008B;
/// Texture unit 0 type.
pub const TEXUNIT0_TYPE: u16 = 0x008E;
/// Lighting toggle.
pub const LIGHTING_ENABLE0: u16 = 0x008F;
/// Texture unit 1 border color.
pub const TEXUNIT1_BORDER_COLOR: u16 = 0x0091;
/// Texture unit 1 dimensions.
pub const TEXUNIT1_DIM: u16 = 0x0092;
/// Texture unit 1 parameters.
pub const TEXUNIT1_PARAM: u16 = 0x0093;
/// Texture unit 1 LOD.
pub const TEXUNIT1_LOD: u16 = 0x0094;
/// Texture unit 1 address.
pub const TEXUNIT1_ADDR: u16 = 0x0095;
/// Texture unit 1 type.
pub const TEXUNIT1_TYPE: u16 = 0x0096;
/// Texture unit 2 border color.
pub const TEXUNIT2_BORDER_COLOR: u16 = 0x0099;
/// Texture unit 2 dimensions.
pub const TEXUNIT2_DIM: u16 = 0x009A;
/// Texture unit 2 parameters.
pub const TEXUNIT2_PARAM: u16 = 0x009B;
/// Texture unit 2 LOD.
pub const TEXUNIT2_LOD: u16 = 0x009C;
/// Texture unit 2 address.
pub const TEXUNIT2_ADDR: u16 = 0x009D;
/// Texture unit 2 type.
pub const TEXUNIT2_TYPE: u16 = 0x009E;
pub const TEXUNIT3_PROCTEX0: u16 = 0x00A8;
pub const TEXUNIT3_PROCTEX1: u16 = 0x00A9;
pub const TEXUNIT3_PROCTEX2: u16 = 0x00AA;
pub const TEXUNIT3_PROCTEX3: u16 = 0x00AB;
pub const TEXUNIT3_PROCTEX4: u16 = 0x000A;
pub const TEXUNIT3_PROCTEX5: u16 = 0x000D;
pub const PROCTEX_LUT: u16 = 0x00AF;
pub const PROCTEX_LUT_DATA0: u16 = 0x00B0;
pub const PROCTEX_LUT_DATA1: u16 = 0x00B1;
pub const PROCTEX_LUT_DATA2: u16 = 0x00B2;
pub const PROCTEX_LUT_DATA3: u16 = 0x00B3;
pub const PROCTEX_LUT_DATA4: u16 = 0x00B4;
pub const PROCTEX_LUT_DATA5: u16 = 0x00B5;
pub const PROCTEX_LUT_DATA6: u16 = 0x00B6;
pub const PROCTEX_LUT_DATA7: u16 = 0x00B7;
/// Texture env 0 source.
pub const TEXENV0_SOURCE: u16 = 0x00C0;
/// Texture env 0 operand.
pub const TEXENV0_OPERAND: u16 = 0x00C1;
/// Texture env 0 combiner.
pub const TEXENV0_COMBINER: u16 = 0x00C2;
/// Texture env 0 color.
pub const TEXENV0_COLOR: u16 = 0x00C3;
/// Texture env 0 scale.
pub const TEXENV0_SCALE: u16 = 0x00C4;
/// Texture env 1 source.
pub const TEXENV1_SOURCE: u16 = 0x00C8;
/// Texture env 1 operand.
pub const TEXENV1_OPERAND: u16 = 0x00C9;
/// Texture env 1 combiner.
pub const TEXENV1_COMBINER: u16 = 0x00CA;
/// Texture env 1 color.
pub const TEXENV1_COLOR: u16 = 0x00CB;
/// Texture env 1 scale.
pub const TEXENV1_SCALE: u16 = 0x00CC;
/// Texture env 2 source.
pub const TEXENV2_SOURCE: u16 = 0x00D0;
/// Texture env 2 operand.
pub const TEXENV2_OPERAND: u16 = 0x00D1;
/// Texture env 2 combiner.
pub const TEXENV2_COMBINER: u16 = 0x00D2;
/// Texture env 2 color.
pub const TEXENV2_COLOR: u16 = 0x00D3;
/// Texture env 2 scale.
pub const TEXENV2_SCALE: u16 = 0x00D4;
/// Texture env 3 source.
pub const TEXENV3_SOURCE: u16 = 0x00D8;
/// Texture env 3 operand.
pub const TEXENV3_OPERAND: u16 = 0x00D9;
/// Texture env 3 combiner.
pub const TEXENV3_COMBINER: u16 = 0x00DA;
/// Texture env 3 color.
pub const TEXENV3_COLOR: u16 = 0x00DB;
/// Texture env 3 scale.
pub const TEXENV3_SCALE: u16 = 0x00DC;
/// Texture env buffer update flag.
pub const TEXENV_UPDATE_BUFFER: u16 = 0x00E0;
pub const FOG_COLOR: u16 = 0x00E1;
pub const GAS_ATTENUATION: u16 = 0x00E4;
pub const GAS_ACCMAX: u16 = 0x00E5;
pub const FOG_LUT_INDEX: u16 = 0x00E6;
pub const FOG_LUT_DATA0: u16 = 0x00E8;
pub const FOG_LUT_DATA1: u16 = 0x00E9;
pub const FOG_LUT_DATA2: u16 = 0x00EA;
pub const FOG_LUT_DATA3: u16 = 0x00EB;
pub const FOG_LUT_DATA4: u16 = 0x00EC;
pub const FOG_LUT_DATA5: u16 = 0x00ED;
pub const FOG_LUT_DATA6: u16 = 0x00EE;
pub const FOG_LUT_DATA7: u16 = 0x00EF;
/// Texture env 4 source.
pub const TEXENV4_SOURCE: u16 = 0x00F0;
/// Texture env 4 operand.
pub const TEXENV4_OPERAND: u16 = 0x00F1;
/// Texture env 4 combiner.
pub const TEXENV4_COMBINER: u16 = 0x00F2;
/// Texture env 4 color.
pub const TEXENV4_COLOR: u16 = 0x00F3;
/// Texture env 4 scale.
pub const TEXENV4_SCALE: u16 = 0x00F4;
/// Texture env 5 source.
pub const TEXENV5_SOURCE: u16 = 0x00F8;
/// Texture env 5 operand.
pub const TEXENV5_OPERAND: u16 = 0x00F9;
/// Texture env 5 combiner.
pub const TEXENV5_COMBINER: u16 = 0x00FA;
/// Texture env 5 color.
pub const TEXENV5_COLOR: u16 = 0x00FB;
/// Texture env 5 scale.
pub const TEXENV5_SCALE: u16 = 0x00FC;
/// Texture env buffer color.
pub const TEXENV_BUFFER_COLOR: u16 = 0x00FD;
/// Configures fragment operation and blend mode.
pub const COLOR_OPERATION: u16 = 0x0100;
/// Blend function configuration.
pub const BLEND_FUNC: u16 = 0x0101;
/// Logical operator configuration.
pub const LOGIC_OP: u16 = 0x0102;
/// Blend color.
pub const BLEND_COLOR: u16 = 0x0103;
/// Alpha test configuration.
pub const FRAGOP_ALPHA_TEST: u16 = 0x0104;
/// Stencil test configuration.
pub const STENCIL_TEST: u16 = 0x0105;
/// Stencil test operation.
pub const STENCIL_OP: u16 = 0x0106;
/// Depth test and color mask configuration.
pub const DEPTH_COLOR_MASK: u16 = 0x0107;
/// Invalidates the frame buffer.
pub const FRAMEBUFFER_INVALIDATE: u16 = 0x0110;
/// Flushes the frame buffer.
pub const FRAMEBUFFER_FLUSH: u16 = 0x0111;
/// Reads from the color buffer.
pub const COLORBUFFER_READ: u16 = 0x0112;
/// Writes to the color buffer.
pub const COLORBUFFER_WRITE: u16 = 0x0113;
/// Reads from the depth buffer.
pub const DEPTHBUFFER_READ: u16 = 0x0114;
/// Writes to the depth buffer.
pub const DEPTHBUFFER_WRITE: u16 = 0x0115;
/// Depth buffer format.
pub const DEPTHBUFFER_FORMAT: u16 = 0x0116;
/// Color buffer format.
pub const COLORBUFFER_FORMAT: u16 = 0x0117;
pub const EARLYDEPTH_TEST2: u16 = 0x0118;
/// Frame buffer block 32.
pub const FRAMEBUFFER_BLOCK32: u16 = 0x011B;
/// Depth buffer location.
pub const DEPTHBUFFER_LOC: u16 = 0x011C;
/// Color buffer location.
pub const COLORBUFFER_LOC: u16 = 0x011D;
/// Frame buffer dimensions.
pub const FRAMEBUFFER_DIM: u16 = 0x011E;
pub const GAS_LIGHT_XY: u16 = 0x0120;
pub const GAS_LIGHT_Z: u16 = 0x0121;
pub const GAS_LIGHT_Z_COLOR: u16 = 0x0122;
pub const GAS_LUT_INDEX: u16 = 0x0123;
pub const GAS_LUT_DATA: u16 = 0x0124;
pub const GAS_ACCMAX_FEEDBACK: u16 = 0x0125;
pub const GAS_DELTAZ_DEPTH: u16 = 0x0126;
pub const FRAGOP_SHADOW: u16 = 0x0130;
/// Light 0 specular lighting.
pub const LIGHT0_SPECULAR0: u16 = 0x0140;
/// Light 0 specular lighting.
pub const LIGHT0_SPECULAR1: u16 = 0x0141;
/// Light 0 diffuse lighting.
pub const LIGHT0_DIFFUSE: u16 = 0x0142;
/// Light 0 ambient lighting.
pub const LIGHT0_AMBIENT: u16 = 0x0143;
/// Light 0 X and Y.
pub const LIGHT0_XY: u16 = 0x0144;
/// Light 0 Z.
pub const LIGHT0_Z: u16 = 0x0145;
/// Light 0 spotlight direction X and Y.
pub const LIGHT0_SPOTDIR_XY: u16 = 0x0146;
/// Light 0 spotlight direction Z.
pub const LIGHT0_SPOTDIR_Z: u16 = 0x0147;
/// Light 0 configuration.
pub const LIGHT0_CONFIG: u16 = 0x0149;
/// Light 0 attenuation bias.
pub const LIGHT0_ATTENUATION_BIAS: u16 = 0x014A;
/// Light 0 attenuation scale.
pub const LIGHT0_ATTENUATION_SCALE: u16 = 0x014B;
/// Light 1 specular lighting.
pub const LIGHT1_SPECULAR0: u16 = 0x0150;
/// Light 1 specular lighting.
pub const LIGHT1_SPECULAR1: u16 = 0x0151;
/// Light 1 diffuse lighting.
pub const LIGHT1_DIFFUSE: u16 = 0x0152;
/// Light 1 ambient lighting.
pub const LIGHT1_AMBIENT: u16 = 0x0153;
/// Light 1 X and Y.
pub const LIGHT1_XY: u16 = 0x0154;
/// Light 1 Z.
pub const LIGHT1_Z: u16 = 0x0155;
/// Light 1 spotlight direction X and Y.
pub const LIGHT1_SPOTDIR_XY: u16 = 0x0156;
/// Light 1 spotlight direction Z.
pub const LIGHT1_SPOTDIR_Z: u16 = 0x0157;
/// Light 1 configuration.
pub const LIGHT1_CONFIG: u16 = 0x0159;
/// Light 1 attenuation bias.
pub const LIGHT1_ATTENUATION_BIAS: u16 = 0x015A;
/// Light 1 attenuation scale.
pub const LIGHT1_ATTENUATION_SCALE: u16 = 0x015B;
/// Light 2 specular lighting.
pub const LIGHT2_SPECULAR0: u16 = 0x0160;
/// Light 2 specular lighting.
pub const LIGHT2_SPECULAR1: u16 = 0x0161;
/// Light 2 diffuse lighting.
pub const LIGHT2_DIFFUSE: u16 = 0x0162;
/// Light 2 ambient lighting.
pub const LIGHT2_AMBIENT: u16 = 0x0163;
/// Light 2 X and Y.
pub const LIGHT2_XY: u16 = 0x0164;
/// Light 2 Z.
pub const LIGHT2_Z: u16 = 0x0165;
/// Light 2 spotlight direction X and Y.
pub const LIGHT2_SPOTDIR_XY: u16 = 0x0166;
/// Light 2 spotlight direction Z.
pub const LIGHT2_SPOTDIR_Z: u16 = 0x0167;
/// Light 2 configuration.
pub const LIGHT2_CONFIG: u16 = 0x0169;
/// Light 2 attenuation bias.
pub const LIGHT2_ATTENUATION_BIAS: u16 = 0x016A;
/// Light 2 attenuation scale.
pub const LIGHT2_ATTENUATION_SCALE: u16 = 0x016B;
/// Light 3 specular lighting.
pub const LIGHT3_SPECULAR0: u16 = 0x0170;
/// Light 3 specular lighting.
pub const LIGHT3_SPECULAR1: u16 = 0x0171;
/// Light 3 diffuse lighting.
pub const LIGHT3_DIFFUSE: u16 = 0x0172;
/// Light 3 ambient lighting.
pub const LIGHT3_AMBIENT: u16 = 0x0173;
/// Light 3 X and Y.
pub const LIGHT3_XY: u16 = 0x0174;
/// Light 3 Z.
pub const LIGHT3_Z: u16 = 0x0175;
/// Light 3 spotlight direction X and Y.
pub const LIGHT3_SPOTDIR_XY: u16 = 0x0176;
/// Light 3 spotlight direction Z.
pub const LIGHT3_SPOTDIR_Z: u16 = 0x0177;
/// Light 3 configuration.
pub const LIGHT3_CONFIG: u16 = 0x0179;
/// Light 3 attenuation bias.
pub const LIGHT3_ATTENUATION_BIAS: u16 = 0x017A;
/// Light 3 attenuation scale.
pub const LIGHT3_ATTENUATION_SCALE: u16 = 0x017B;
/// Light 4 specular lighting.
pub const LIGHT4_SPECULAR0: u16 = 0x0180;
/// Light 4 specular lighting.
pub const LIGHT4_SPECULAR1: u16 = 0x0181;
/// Light 4 diffuse lighting.
pub const LIGHT4_DIFFUSE: u16 = 0x0182;
/// Light 4 ambient lighting.
pub const LIGHT4_AMBIENT: u16 = 0x0183;
/// Light 4 X and Y.
pub const LIGHT4_XY: u16 = 0x0184;
/// Light 4 Z.
pub const LIGHT4_Z: u16 = 0x0185;
/// Light 4 spotlight direction X and Y.
pub const LIGHT4_SPOTDIR_XY: u16 = 0x0186;
/// Light 4 spotlight direction Z.
pub const LIGHT4_SPOTDIR_Z: u16 = 0x0187;
/// Light 4 configuration.
pub const LIGHT4_CONFIG: u16 = 0x0189;
/// Light 4 attenuation bias.
pub const LIGHT4_ATTENUATION_BIAS: u16 = 0x018A;
/// Light 4 attenuation scale.
pub const LIGHT4_ATTENUATION_SCALE: u16 = 0x018B;
/// Light 5 specular lighting.
pub const LIGHT5_SPECULAR0: u16 = 0x0190;
/// Light 5 specular lighting.
pub const LIGHT5_SPECULAR1: u16 = 0x0191;
/// Light 5 diffuse lighting.
pub const LIGHT5_DIFFUSE: u16 = 0x0192;
/// Light 5 ambient lighting.
pub const LIGHT5_AMBIENT: u16 = 0x0193;
/// Light 5 X and Y.
pub const LIGHT5_XY: u16 = 0x0194;
/// Light 5 Z.
pub const LIGHT5_Z: u16 = 0x0195;
/// Light 5 spotlight direction X and Y.
pub const LIGHT5_SPOTDIR_XY: u16 = 0x0196;
/// Light 5 spotlight direction Z.
pub const LIGHT5_SPOTDIR_Z: u16 = 0x0197;
/// Light 5 configuration.
pub const LIGHT5_CONFIG: u16 = 0x0199;
/// Light 5 attenuation bias.
pub const LIGHT5_ATTENUATION_BIAS: u16 = 0x019A;
/// Light 5 attenuation scale.
pub const LIGHT5_ATTENUATION_SCALE: u16 = 0x019B;
/// Light 6 specular lighting.
pub const LIGHT6_SPECULAR0: u16 = 0x01A0;
/// Light 6 specular lighting.
pub const LIGHT6_SPECULAR1: u16 = 0x01A1;
/// Light 6 diffuse lighting.
pub const LIGHT6_DIFFUSE: u16 = 0x01A2;
/// Light 6 ambient lighting.
pub const LIGHT6_AMBIENT: u16 = 0x01A3;
/// Light 6 X and Y.
pub const LIGHT6_XY: u16 = 0x01A4;
/// Light 6 Z.
pub const LIGHT6_Z: u16 = 0x01A5;
/// Light 6 spotlight direction X and Y.
pub const LIGHT6_SPOTDIR_XY: u16 = 0x01A6;
/// Light 6 spotlight direction Z.
pub const LIGHT6_SPOTDIR_Z: u16 = 0x01A7;
/// Light 6 configuration.
pub const LIGHT6_CONFIG: u16 = 0x01A9;
/// Light 6 attenuation bias.
pub const LIGHT6_ATTENUATION_BIAS: u16 = 0x01AA;
/// Light 6 attenuation scale.
pub const LIGHT6_ATTENUATION_SCALE: u16 = 0x01AB;
/// Light 7 specular lighting.
pub const LIGHT7_SPECULAR0: u16 = 0x01B0;
/// Light 7 specular lighting.
pub const LIGHT7_SPECULAR1: u16 = 0x01B1;
/// Light 7 diffuse lighting.
pub const LIGHT7_DIFFUSE: u16 = 0x01B2;
/// Light 7 ambient lighting.
pub const LIGHT7_AMBIENT: u16 = 0x01B3;
/// Light 7 X and Y.
pub const LIGHT7_XY: u16 = 0x01B4;
/// Light 7 Z.
pub const LIGHT7_Z: u16 = 0x01B5;
/// Light 7 spotlight direction X and Y.
pub const LIGHT7_SPOTDIR_XY: u16 = 0x01B6;
/// Light 7 spotlight direction Z.
pub const LIGHT7_SPOTDIR_Z: u16 = 0x01B7;
/// Light 7 configuration.
pub const LIGHT7_CONFIG: u16 = 0x01B9;
/// Light 7 attenuation bias.
pub const LIGHT7_ATTENUATION_BIAS: u16 = 0x01BA;
/// Light 7 attenuation scale.
pub const LIGHT7_ATTENUATION_SCALE: u16 = 0x01BB;
/// Ambient lighting.
pub const LIGHTING_AMBIENT: u16 = 0x01C0;
/// Number of lights.
pub const LIGHTING_NUM_LIGHTS: u16 = 0x01C2;
/// Lighting configuration.
pub const LIGHTING_CONFIG0: u16 = 0x01C3;
/// Lighting configuration.
pub const LIGHTING_CONFIG1: u16 = 0x01C4;
/// LUT index.
pub const LIGHTING_LUT_INDEX: u16 = 0x01C5;
/// Lighting toggle.
pub const LIGHTING_ENABLE1: u16 = 0x01C6;
/// LUT data 0.
pub const LIGHTING_LUT_DATA0: u16 = 0x01C8;
/// LUT data 1.
pub const LIGHTING_LUT_DATA1: u16 = 0x01C9;
/// LUT data 2.
pub const LIGHTING_LUT_DATA2: u16 = 0x01CA;
/// LUT data 3.
pub const LIGHTING_LUT_DATA3: u16 = 0x01CB;
/// LUT data 4.
pub const LIGHTING_LUT_DATA4: u16 = 0x01CC;
/// LUT data 5.
pub const LIGHTING_LUT_DATA5: u16 = 0x01CD;
/// LUT data 6.
pub const LIGHTING_LUT_DATA6: u16 = 0x01CE;
/// LUT data 7.
pub const LIGHTING_LUT_DATA7: u16 = 0x01CF;
/// LUT input abs.
pub const LIGHTING_LUTINPUT_ABS: u16 = 0x01D0;
/// LUT input selector.
pub const LIGHTING_LUTINPUT_SELECT: u16 = 0x01D1;
/// LUT input scale.
pub const LIGHTING_LUTINPUT_SCALE: u16 = 0x01D2;
/// Light permutation.
pub const LIGHTING_LIGHT_PERMUTATION: u16 = 0x01D9;
/// Attribute buffers location.
pub const ATTRIBBUFFERS_LOC: u16 = 0x0200;
/// Attribute buffers format low.
pub const ATTRIBBUFFERS_FORMAT_LOW: u16 = 0x0201;
/// Attribute buffers format high.
pub const ATTRIBBUFFERS_FORMAT_HIGH: u16 = 0x0202;
/// Attribute buffers 0 offset.
pub const ATTRIBBUFFER0_OFFSET: u16 = 0x0203;
/// Attribute buffers 0 configuration.
pub const ATTRIBBUFFER0_CONFIG1: u16 = 0x0204;
/// Attribute buffers 0 configuration.
pub const ATTRIBBUFFER0_CONFIG2: u16 = 0x0205;
/// Attribute buffers 1 offset.
pub const ATTRIBBUFFER1_OFFSET: u16 = 0x0206;
/// Attribute buffers 1 configuration.
pub const ATTRIBBUFFER1_CONFIG1: u16 = 0x0207;
/// Attribute buffers 1 configuration.
pub const ATTRIBBUFFER1_CONFIG2: u16 = 0x0208;
/// Attribute buffers 2 offset.
pub const ATTRIBBUFFER2_OFFSET: u16 = 0x0209;
/// Attribute buffers 2 configuration.
pub const ATTRIBBUFFER2_CONFIG1: u16 = 0x020A;
/// Attribute buffers 2 configuration.
pub const ATTRIBBUFFER2_CONFIG2: u16 = 0x020B;
/// Attribute buffers 3 offset.
pub const ATTRIBBUFFER3_OFFSET: u16 = 0x020C;
/// Attribute buffers 3 configuration.
pub const ATTRIBBUFFER3_CONFIG1: u16 = 0x020D;
/// Attribute buffers 3 configuration.
pub const ATTRIBBUFFER3_CONFIG2: u16 = 0x020E;
/// Attribute buffers 4 offset.
pub const ATTRIBBUFFER4_OFFSET: u16 = 0x020F;
/// Attribute buffers 4 configuration.
pub const ATTRIBBUFFER4_CONFIG1: u16 = 0x0210;
/// Attribute buffers 4 configuration.
pub const ATTRIBBUFFER4_CONFIG2: u16 = 0x0211;
/// Attribute buffers 5 offset.
pub const ATTRIBBUFFER5_OFFSET: u16 = 0x0212;
/// Attribute buffers 5 configuration.
pub const ATTRIBBUFFER5_CONFIG1: u16 = 0x0213;
/// Attribute buffers 5 configuration.
pub const ATTRIBBUFFER5_CONFIG2: u16 = 0x0214;
/// Attribute buffers 6 offset.
pub const ATTRIBBUFFER6_OFFSET: u16 = 0x0215;
/// Attribute buffers 6 configuration.
pub const ATTRIBBUFFER6_CONFIG1: u16 = 0x0216;
/// Attribute buffers 6 configuration.
pub const ATTRIBBUFFER6_CONFIG2: u16 = 0x0217;
/// Attribute buffers 7 offset.
pub const ATTRIBBUFFER7_OFFSET: u16 = 0x0218;
/// Attribute buffers 7 configuration.
pub const ATTRIBBUFFER7_CONFIG1: u16 = 0x0219;
/// Attribute buffers 7 configuration.
pub const ATTRIBBUFFER7_CONFIG2: u16 = 0x021A;
/// Attribute buffers 8 offset.
pub const ATTRIBBUFFER8_OFFSET: u16 = 0x021B;
/// Attribute buffers 8 configuration.
pub const ATTRIBBUFFER8_CONFIG1: u16 = 0x021C;
/// Attribute buffers 8 configuration.
pub const ATTRIBBUFFER8_CONFIG2: u16 = 0x021D;
/// Attribute buffers 9 offset.
pub const ATTRIBBUFFER9_OFFSET: u16 = 0x021E;
/// Attribute buffers 9 configuration.
pub const ATTRIBBUFFER9_CONFIG1: u16 = 0x021F;
/// Attribute buffers 9 configuration.
pub const ATTRIBBUFFER9_CONFIG2: u16 = 0x0220;
/// Attribute buffers A offset.
pub const ATTRIBBUFFERA_OFFSET: u16 = 0x0221;
/// Attribute buffers A configuration.
pub const ATTRIBBUFFERA_CONFIG1: u16 = 0x0222;
/// Attribute buffers A configuration.
pub const ATTRIBBUFFERA_CONFIG2: u16 = 0x0223;
/// Attribute buffers B offset.
pub const ATTRIBBUFFERB_OFFSET: u16 = 0x0224;
/// Attribute buffers B configuration.
pub const ATTRIBBUFFERB_CONFIG1: u16 = 0x0225;
/// Attribute buffers B configuration.
pub const ATTRIBBUFFERB_CONFIG2: u16 = 0x0226;
/// Index buffer configuration.
pub const INDEXBUFFER_CONFIG: u16 = 0x0227;
/// Number of vertices.
pub const NUMVERTICES: u16 = 0x0228;
/// Geometry stage configuration.
pub const GEOSTAGE_CONFIG: u16 = 0x0229;
/// Vertex offset.
pub const VERTEX_OFFSET: u16 = 0x022A;
pub const POST_VERTEX_CACHE_NUM: u16 = 0x022D;
/// Draw arrays trigger.
pub const DRAWARRAYS: u16 = 0x022E;
/// Draw arrays elements.
pub const DRAWELEMENTS: u16 = 0x022F;
pub const VTX_FUNC: u16 = 0x0231;
/// Fixed attribute index.
pub const FIXEDATTRIB_INDEX: u16 = 0x0232;
/// Fixed attribute data 0.
pub const FIXEDATTRIB_DATA0: u16 = 0x0233;
/// Fixed attribute data 1.
pub const FIXEDATTRIB_DATA1: u16 = 0x0234;
/// Fixed attribute data 2.
pub const FIXEDATTRIB_DATA2: u16 = 0x0235;
/// Command buffer size 0.
pub const CMDBUF_SIZE0: u16 = 0x0238;
/// Command buffer size 1.
pub const CMDBUF_SIZE1: u16 = 0x0239;
/// Command buffer address 0.
pub const CMDBUF_ADDR0: u16 = 0x023A;
/// Command buffer address 1.
pub const CMDBUF_ADDR1: u16 = 0x023B;
/// Command buffer jump 0.
pub const CMDBUF_JUMP0: u16 = 0x023C;
/// Command buffer jump 1.
pub const CMDBUF_JUMP1: u16 = 0x023D;
pub const VSH_NUM_ATTR: u16 = 0x0242;
pub const VSH_COM_MODE: u16 = 0x0244;
pub const START_DRAW_FUNC0: u16 = 0x0245;
pub const VSH_OUTMAP_TOTAL1: u16 = 0x024A;
pub const VSH_OUTMAP_TOTAL2: u16 = 0x0251;
pub const GSH_MISC0: u16 = 0x0252;
pub const GEOSTAGE_CONFIG2: u16 = 0x0253;
pub const GSH_MISC1: u16 = 0x0254;
/// Primitive configuration.
pub const PRIMITIVE_CONFIG: u16 = 0x025E;
/// Restart primitive flag.
pub const RESTART_PRIMITIVE: u16 = 0x025F;
/// Geometry shader bool uniforms.
pub const GSH_BOOLUNIFORM: u16 = 0x0280;
/// Geometry shader integer uniform 0.
pub const GSH_INTUNIFORM_I0: u16 = 0x0281;
/// Geometry shader integer uniform 1.
pub const GSH_INTUNIFORM_I1: u16 = 0x0282;
/// Geometry shader integer uniform 2.
pub const GSH_INTUNIFORM_I2: u16 = 0x0283;
/// Geometry shader integer uniform 3.
pub const GSH_INTUNIFORM_I3: u16 = 0x0284;
/// Geometry shader input buffer configuration.
pub const GSH_INPUTBUFFER_CONFIG: u16 = 0x0289;
/// Geometry shader entry point.
pub const GSH_ENTRYPOINT: u16 = 0x028A;
/// Geometry shader attribute permutations low.
pub const GSH_ATTRIBUTES_PERMUTATION_LOW: u16 = 0x028B;
/// Geometry shader attribute permutations high.
pub const GSH_ATTRIBUTES_PERMUTATION_HIGH: u16 = 0x028C;
/// Geometry shader output map mask.
pub const GSH_OUTMAP_MASK: u16 = 0x028D;
/// Geometry shader code transfer end trigger.
pub const GSH_CODETRANSFER_END: u16 = 0x028F;
/// Geometry shader float uniform configuration.
pub const GSH_FLOATUNIFORM_CONFIG: u16 = 0x0290;
/// Geometry shader float uniform data.
pub const GSH_FLOATUNIFORM_DATA: u16 = 0x0291;
/// Geometry shader code transfer configuration.
pub const GSH_CODETRANSFER_CONFIG: u16 = 0x029B;
/// Geometry shader code transfer data.
pub const GSH_CODETRANSFER_DATA: u16 = 0x029C;
/// Geometry shader operand description configuration.
pub const GSH_OPDESCS_CONFIG: u16 = 0x02A5;
/// Geometry shader operand description data.
pub const GSH_OPDESCS_DATA: u16 = 0x02A6;
/// Vertex shader bool uniforms.
pub const VSH_BOOLUNIFORM: u16 = 0x02B0;
/// Vertex shader integer uniform 0.
pub const VSH_INTUNIFORM_I0: u16 = 0x02B1;
/// Vertex shader integer uniform 1.
pub const VSH_INTUNIFORM_I1: u16 = 0x02B2;
/// Vertex shader integer uniform 2.
pub const VSH_INTUNIFORM_I2: u16 = 0x02B3;
/// Vertex shader integer uniform 3.
pub const VSH_INTUNIFORM_I3: u16 = 0x02B4;
/// Vertex shader input buffer configuration.
pub const VSH_INPUTBUFFER_CONFIG: u16 = 0x02B9;
/// Vertex shader entry point.
pub const VSH_ENTRYPOINT: u16 = 0x02BA;
/// Vertex shader attribute permutations low.
pub const VSH_ATTRIBUTES_PERMUTATION_LOW: u16 = 0x02BB;
/// Vertex shader attribute permutations high.
pub const VSH_ATTRIBUTES_PERMUTATION_HIGH: u16 = 0x02BC;
/// Vertex shader output map mask.
pub const VSH_OUTMAP_MASK: u16 = 0x02BD;
/// Vertex shader code transfer end trigger.
pub const VSH_CODETRANSFER_END: u16 = 0x02BF;
/// Vertex shader float uniform configuration.
pub const VSH_FLOATUNIFORM_CONFIG: u16 = 0x02C0;
/// Vertex shader float uniform data.
pub const VSH_FLOATUNIFORM_DATA: u16 = 0x02C1;
/// Vertex shader code transfer configuration.
pub const VSH_CODETRANSFER_CONFIG: u16 = 0x02CB;
/// Vertex shader code transfer data.
pub const VSH_CODETRANSFER_DATA: u16 = 0x02CC;
/// Vertex shader operand description configuration.
pub const VSH_OPDESCS_CONFIG: u16 = 0x02D5;
/// Vertex shader operand description data.
pub const VSH_OPDESCS_DATA: u16 = 0x02D6;
