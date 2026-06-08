package com.auroragba

import android.app.Activity
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.os.Bundle
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowManager
import android.widget.FrameLayout
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicReference

/**
 * Tela única do emulador: uma [SurfaceView] mostra o framebuffer e um
 * [ControlsView] por cima trata o toque. Uma thread dedicada roda o loop de
 * emulação+render; **todo** acesso ao ponteiro nativo do [Gba][NativeBridge]
 * acontece nessa thread. A UI passa input (máscara de botões) e a ROM escolhida
 * por estruturas atômicas.
 */
class MainActivity : Activity(), SurfaceHolder.Callback {

    private companion object {
        const val W = 240
        const val H = 160
        const val OPEN_ROM = 1
        val MATCH = FrameLayout.LayoutParams.MATCH_PARENT
    }

    private lateinit var surface: SurfaceView
    private val buttonMask = AtomicInteger(0)
    private val pendingRom = AtomicReference<ByteArray?>(null)

    // Ponteiro nativo: criado pela thread de render, liberado em onDestroy (vive
    // entre destruições da surface — pause/resume não reseta o jogo).
    @Volatile private var handle = 0L
    @Volatile private var running = false
    @Volatile private var romLoaded = false
    private var renderThread: Thread? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        val root = FrameLayout(this)
        surface = SurfaceView(this)
        surface.holder.addCallback(this)
        root.addView(surface, FrameLayout.LayoutParams(MATCH, MATCH))

        val controls = ControlsView(this)
        controls.onMask = { buttonMask.set(it) }
        controls.onOpenRom = { openRomPicker() }
        root.addView(controls, FrameLayout.LayoutParams(MATCH, MATCH))

        setContentView(root)

        if (savedInstanceState == null) openRomPicker()
    }

    private fun openRomPicker() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            type = "*/*"
        }
        startActivityForResult(intent, OPEN_ROM)
    }

    @Deprecated("API clássica; suficiente para este app de tela única")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == OPEN_ROM && resultCode == RESULT_OK) {
            val uri = data?.data ?: return
            val bytes = contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return
            pendingRom.set(bytes)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        stopRenderThread()
        if (handle != 0L) {
            NativeBridge.destroy(handle)
            handle = 0L
        }
    }

    // ── SurfaceHolder.Callback ───────────────────────────────────────────────
    override fun surfaceCreated(holder: SurfaceHolder) = startRenderThread()
    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {}
    override fun surfaceDestroyed(holder: SurfaceHolder) = stopRenderThread()

    private fun startRenderThread() {
        if (renderThread != null) return
        running = true
        renderThread = Thread(::renderLoop, "emu").also { it.start() }
    }

    private fun stopRenderThread() {
        running = false
        renderThread?.join()
        renderThread = null
    }

    private fun renderLoop() {
        if (handle == 0L) handle = NativeBridge.create()
        val buf = ByteBuffer.allocateDirect(W * H * 4).order(ByteOrder.nativeOrder())
        val bitmap = Bitmap.createBitmap(W, H, Bitmap.Config.ARGB_8888)
        val paint = Paint().apply { isFilterBitmap = false }
        val src = Rect(0, 0, W, H)
        val frameNanos = (1_000_000_000.0 / 59.7275).toLong()

        while (running) {
            val start = System.nanoTime()

            pendingRom.getAndSet(null)?.let {
                NativeBridge.loadRom(handle, it)
                romLoaded = true
            }
            if (romLoaded) {
                NativeBridge.setButtons(handle, buttonMask.get())
                buf.clear()
                NativeBridge.renderFrame(handle, buf)
                buf.rewind()
                bitmap.copyPixelsFromBuffer(buf)
            }

            val canvas = surface.holder.lockCanvas()
            if (canvas == null) {
                Thread.sleep(8)
                continue
            }
            try {
                canvas.drawColor(Color.BLACK)
                if (romLoaded) {
                    canvas.drawBitmap(bitmap, src, fitRect(canvas.width, canvas.height), paint)
                }
            } finally {
                surface.holder.unlockCanvasAndPost(canvas)
            }

            val sleep = frameNanos - (System.nanoTime() - start)
            if (sleep > 0) Thread.sleep(sleep / 1_000_000, (sleep % 1_000_000).toInt())
        }
    }

    /** Retângulo de destino preservando o aspecto 240:160, centralizado. */
    private fun fitRect(cw: Int, ch: Int): RectF {
        val scale = minOf(cw.toFloat() / W, ch.toFloat() / H)
        val w = W * scale
        val h = H * scale
        val left = (cw - w) / 2f
        val top = (ch - h) / 2f
        return RectF(left, top, left + w, top + h)
    }
}
