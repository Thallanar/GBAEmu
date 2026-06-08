package com.auroragba

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.view.MotionEvent
import android.view.View

/** Bits dos botões (mesma ordem do KEYINPUT e da ponte JNI). */
object Btn {
    const val A = 1 shl 0
    const val B = 1 shl 1
    const val SELECT = 1 shl 2
    const val START = 1 shl 3
    const val RIGHT = 1 shl 4
    const val LEFT = 1 shl 5
    const val UP = 1 shl 6
    const val DOWN = 1 shl 7
    const val R = 1 shl 8
    const val L = 1 shl 9
}

/**
 * Overlay transparente com os controles na tela. A cada toque recomputa a máscara
 * de botões (multi-touch) a partir de TODOS os ponteiros ativos e chama
 * [onMask]. Desenha D-pad, A/B, Start/Select e L/R.
 */
class ControlsView(context: Context) : View(context) {
    var onMask: (Int) -> Unit = {}
    var onMenu: () -> Unit = {}

    private val d = resources.displayMetrics.density
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(70, 255, 255, 255) }
    private val active = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(140, 120, 200, 255) }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(200, 255, 255, 255)
        textAlign = Paint.Align.CENTER
        textSize = 16 * d
    }

    private var mask = 0

    // ── Geometria (recalculada conforme o tamanho da view) ───────────────────
    private val pad get() = 24 * d
    private val dpadCx get() = pad + 96 * d
    private val dpadCy get() = height - pad - 96 * d
    private val arm get() = 64 * d // meio-comprimento do braço do D-pad
    private val thick get() = 46 * d // largura do braço
    private val abR get() = 44 * d
    private val aCx get() = width - pad - 64 * d
    private val aCy get() = height - pad - 120 * d
    private val bCx get() = width - pad - 152 * d
    private val bCy get() = height - pad - 64 * d
    private val seSize get() = 30 * d
    private val startCx get() = width / 2f + 60 * d
    private val selCx get() = width / 2f - 60 * d
    private val seCy get() = height - pad - 24 * d
    private val lrW get() = 120 * d
    private val lrH get() = 48 * d

    // Botão de menu (topo, centro): ROM, save states e opções futuras.
    private fun menuRect() = RectF(width / 2f - 60 * d, pad, width / 2f + 60 * d, pad + lrH)

    /** Botões pressionados por um ponteiro na posição (x, y). */
    private fun hit(x: Float, y: Float): Int {
        var m = 0
        // D-pad: dentro da caixa do "+", direção pelo deslocamento do centro.
        val dx = x - dpadCx
        val dy = y - dpadCy
        val reach = arm + thick / 2f
        if (kotlin.math.abs(dx) <= reach && kotlin.math.abs(dy) <= reach) {
            val dz = 16 * d
            if (dx < -dz) m = m or Btn.LEFT
            if (dx > dz) m = m or Btn.RIGHT
            if (dy < -dz) m = m or Btn.UP
            if (dy > dz) m = m or Btn.DOWN
        }
        if (dist(x, y, aCx, aCy) <= abR) m = m or Btn.A
        if (dist(x, y, bCx, bCy) <= abR) m = m or Btn.B
        if (inPill(x, y, startCx, seCy)) m = m or Btn.START
        if (inPill(x, y, selCx, seCy)) m = m or Btn.SELECT
        if (x <= pad + lrW && y <= pad + lrH) m = m or Btn.L
        if (x >= width - pad - lrW && y <= pad + lrH) m = m or Btn.R
        return m
    }

    private fun dist(x: Float, y: Float, cx: Float, cy: Float) =
        kotlin.math.hypot((x - cx).toDouble(), (y - cy).toDouble()).toFloat()

    private fun inPill(x: Float, y: Float, cx: Float, cy: Float) =
        x >= cx - 44 * d && x <= cx + 44 * d && y >= cy - seSize && y <= cy + seSize

    override fun onTouchEvent(event: MotionEvent): Boolean {
        // Toque novo no botão de menu (topo/centro) abre o menu e não vira input.
        if (event.actionMasked == MotionEvent.ACTION_DOWN ||
            event.actionMasked == MotionEvent.ACTION_POINTER_DOWN
        ) {
            val i = event.actionIndex
            if (menuRect().contains(event.getX(i), event.getY(i))) {
                onMenu()
                return true
            }
        }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN,
            MotionEvent.ACTION_POINTER_DOWN,
            MotionEvent.ACTION_MOVE,
            MotionEvent.ACTION_POINTER_UP,
            MotionEvent.ACTION_UP,
            MotionEvent.ACTION_CANCEL -> {
                var m = 0
                if (event.actionMasked != MotionEvent.ACTION_UP &&
                    event.actionMasked != MotionEvent.ACTION_CANCEL
                ) {
                    val up = event.actionMasked == MotionEvent.ACTION_POINTER_UP
                    val skip = if (up) event.actionIndex else -1
                    for (i in 0 until event.pointerCount) {
                        if (i == skip) continue
                        m = m or hit(event.getX(i), event.getY(i))
                    }
                }
                if (m != mask) {
                    mask = m
                    onMask(m)
                    invalidate()
                }
            }
        }
        return true
    }

    override fun onDraw(canvas: Canvas) {
        // D-pad (+).
        fun arm(r: RectF, on: Boolean) = canvas.drawRoundRect(r, 10 * d, 10 * d, if (on) active else fill)
        arm(RectF(dpadCx - thick / 2, dpadCy - arm, dpadCx + thick / 2, dpadCy + arm), false)
        arm(RectF(dpadCx - arm, dpadCy - thick / 2, dpadCx + arm, dpadCy + thick / 2), false)
        highlight(canvas)

        circle(canvas, aCx, aCy, abR, "A", mask and Btn.A != 0)
        circle(canvas, bCx, bCy, abR, "B", mask and Btn.B != 0)
        pill(canvas, startCx, seCy, "START", mask and Btn.START != 0)
        pill(canvas, selCx, seCy, "SEL", mask and Btn.SELECT != 0)
        rect(canvas, pad, pad, pad + lrW, pad + lrH, "L", mask and Btn.L != 0)
        rect(canvas, width - pad - lrW, pad, width - pad, pad + lrH, "R", mask and Btn.R != 0)
        val o = menuRect()
        rect(canvas, o.left, o.top, o.right, o.bottom, "☰ MENU", false)
    }

    /** Realça a(s) direção(ões) ativa(s) do D-pad. */
    private fun highlight(canvas: Canvas) {
        val hl = active
        val a = arm
        val t = thick
        if (mask and Btn.UP != 0) {
            canvas.drawRoundRect(RectF(dpadCx - t / 2, dpadCy - a, dpadCx + t / 2, dpadCy), 10 * d, 10 * d, hl)
        }
        if (mask and Btn.DOWN != 0) {
            canvas.drawRoundRect(RectF(dpadCx - t / 2, dpadCy, dpadCx + t / 2, dpadCy + a), 10 * d, 10 * d, hl)
        }
        if (mask and Btn.LEFT != 0) {
            canvas.drawRoundRect(RectF(dpadCx - a, dpadCy - t / 2, dpadCx, dpadCy + t / 2), 10 * d, 10 * d, hl)
        }
        if (mask and Btn.RIGHT != 0) {
            canvas.drawRoundRect(RectF(dpadCx, dpadCy - t / 2, dpadCx + a, dpadCy + t / 2), 10 * d, 10 * d, hl)
        }
    }

    private fun circle(canvas: Canvas, cx: Float, cy: Float, r: Float, label: String, on: Boolean) {
        canvas.drawCircle(cx, cy, r, if (on) active else fill)
        canvas.drawText(label, cx, cy + 6 * d, text)
    }

    private fun pill(canvas: Canvas, cx: Float, cy: Float, label: String, on: Boolean) {
        canvas.drawRoundRect(
            RectF(cx - 44 * d, cy - seSize, cx + 44 * d, cy + seSize),
            seSize, seSize, if (on) active else fill,
        )
        canvas.drawText(label, cx, cy + 5 * d, text)
    }

    private fun rect(canvas: Canvas, l: Float, t: Float, r: Float, b: Float, label: String, on: Boolean) {
        canvas.drawRoundRect(RectF(l, t, r, b), 10 * d, 10 * d, if (on) active else fill)
        canvas.drawText(label, (l + r) / 2, (t + b) / 2 + 6 * d, text)
    }
}
