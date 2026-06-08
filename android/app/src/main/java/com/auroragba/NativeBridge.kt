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

    /**
     * Copia até `out.size` amostras (i16 intercaladas L,R a 32768 Hz) do APU pro
     * array e devolve quantas copiou. Escreve direto num `AudioTrack` 32768 Hz
     * estéreo.
     */
    external fun drainAudio(handle: Long, out: ShortArray): Int

    // ── saves (.sav + estados) ───────────────────────────────────────────────

    /** Game code (4 chars do cabeçalho), chave dos arquivos de save. */
    external fun gameCode(handle: Long): String

    /** O jogo carregado tem memória de save (.sav)? */
    external fun hasSave(handle: Long): Boolean

    /** O backup mudou desde a última gravação? */
    external fun backupDirty(handle: Long): Boolean

    /** Marca o backup como gravado (chamar após escrever o `.sav`). */
    external fun clearBackupDirty(handle: Long)

    /** Cópia dos bytes do backup (.sav) pra gravar em disco. */
    external fun saveBackup(handle: Long): ByteArray

    /** Carrega um `.sav` do disco; `true` se o tamanho bateu. */
    external fun loadBackup(handle: Long, data: ByteArray): Boolean

    /** Serializa o estado completo (save state) pra gravar em disco. */
    external fun saveState(handle: Long): ByteArray

    /** Restaura um save state por cima do jogo atual; `true` se válido. */
    external fun loadState(handle: Long, data: ByteArray): Boolean
}
