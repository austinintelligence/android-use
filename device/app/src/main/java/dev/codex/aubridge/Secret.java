package dev.codex.aubridge;

/** Decides which fields hold secrets, from the Android inputType bits. */
final class Secret {
    private Secret() {}

    /** True for text, web, and number password variations (visible passwords stay readable). */
    static boolean password(int inputType) {
        int variation = inputType & 0xff0;
        switch (inputType & 0xf) {
            case 1:
                return variation == 0x80 || variation == 0xe0;
            case 2:
                return variation == 0x10;
            default:
                return false;
        }
    }
}
