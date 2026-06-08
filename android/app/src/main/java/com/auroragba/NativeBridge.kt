package com.auroragba

import java.nio.ByteBuffer

/**
 * Ponte JNI pro core do emulador (crate `auroragba-android`, `libauroragba_android.so`).
 *
 * IMPORTANTE: todas as funções operam sobre o mesmo ponteiro `handle` e **não**
 * são thread-safe entre si — devem ser chamadas sempre pela mesma thread (a
 * thread de emulação). A UI comunica input/ROM por estruturas atômicas.
 */
object NativeBridge {
    init {
        System.loadLibrary("auroragba_android")
    }

    /** Cria uma instância do emulador e devolve um ponteiro opaco. */
    external fun create(): Long

    /** Libera a instância. */
    external fun destroy(handle: Long)

    /** Carrega uma ROM (bytes) e faz o direct boot. */
    external fun loadRom(handle: Long, rom: ByteArray)

    /**
     * Roda um frame e copia o framebuffer (RGBA8 240×160) pro [buffer] direto.
     * O buffer precisa ser `ByteBuffer.allocateDirect(240*160*4)`.
     */
    external fun renderFrame(handle: Long, buffer: ByteBuffer)

    /** Atualiza os botões (bits do KEYINPUT: 0=A,1=B,2=Sel,3=Start,4=R,5=L,6=Up,7=Down,8=R,9=L). */
    external fun setButtons(handle: Long, mask: Int)
}
