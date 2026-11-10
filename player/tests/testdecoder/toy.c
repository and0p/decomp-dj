/* A decoder plugin in C for "toy8", a made-up instruction set, for the player's tests:
 * it checks the C side of decoders/include/dj_decoder.h. Opcodes (CPU addresses are 16-bit):
 *   00        NOP
 *   10 rr     BEQ: a branch to pc + 2 + rr (signed)
 *   20 lo hi  CALL abs (listed as "CALL")
 *   30 lo hi  JMP abs
 *   40        RET
 *   50        HALT
 *   60 rr     LDL: reads 2 bytes of data at pc + 2 + rr
 *   EE        a broken answer (an unknown kind), which the player must refuse
 *   FF        not an instruction
 *   others    1-byte instructions */
#include "../../../decoders/include/dj_decoder.h"

static void toy_decode(const uint8_t* b, size_t n, uint64_t pc, uint32_t mode, struct dj_instruction* out) {
	(void) mode;
	if (n == 0) {
		return;
	}
	switch (b[0]) {
	case 0x10:
		out->length = 2;
		out->kind = DJ_JUMP;
		out->conditional = 1;
		out->has_target = 1;
		out->target = (pc + 2 + (int8_t) (n > 1 ? b[1] : 0)) & 0xFFFF;
		break;
	case 0x20:
	case 0x30:
		out->length = 3;
		out->kind = b[0] == 0x20 ? DJ_CALL : DJ_JUMP;
		out->has_target = 1;
		out->target = n > 2 ? (uint64_t) (b[1] | (b[2] << 8)) : 0;
		out->site = b[0] == 0x20 ? "CALL" : "JMP";
		break;
	case 0x40:
		out->length = 1;
		out->kind = DJ_RETURN;
		break;
	case 0x50:
		out->length = 1;
		out->kind = DJ_STOP;
		break;
	case 0x60:
		out->length = 2;
		out->data_ref = (pc + 2 + (int8_t) (n > 1 ? b[1] : 0)) & 0xFFFF;
		out->data_ref_length = 2;
		break;
	case 0xEE:
		out->length = 1;
		out->kind = 9;
		break;
	case 0xFF:
		break;
	default:
		out->length = 1;
	}
}

static const struct dj_decoder toy = {
	DJ_DECODER_API, sizeof(struct dj_decoder), "toy8", 3, 0, {0}, NULL, NULL, toy_decode,
};

static const struct dj_decoder* const all[] = { &toy };

const struct dj_decoder* const* dj_decoders(size_t* count) {
	*count = 1;
	return all;
}
