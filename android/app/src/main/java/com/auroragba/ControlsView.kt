package com.auroragba

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import android.os.Build
import android.os.VibrationEffect
import android.os.Vibrator
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

/** Modelo do direcional na tela (persistido pela chave). */
enum class PadModel(val key: String) {
    /** D-pad em cruz (o layout original). */
    CLASSIC("classic"),
    /** Stick virtual; o GBA é digital, então vira 8 direções. */
    STICK("stick"),
    /** Quatro setas separadas, como no controle do PlayStation. */
    ARROWS("arrows");

    companion object {
        fun from(key: String?) = values().firstOrNull { it.key == key } ?: CLASSIC
    }
}

/**
 * Overlay transparente com os controles na tela. A cada toque recomputa a máscara
 * de botões (multi-touch) a partir de TODOS os ponteiros ativos e chama
 * [onMask]. Desenha o direcional ([model]), A/B, Start/Select, L/R e o Menu.
 */
class ControlsView(context: Context) : View(context) {
    private companion object {
        // Razão mínima entre a componente menor e a maior do toque pra contar
        // como diagonal deliberada (≈ tan 26,5° → cardeais cobrem ±26,5°, bem
        // mais largos que as diagonais). Abaixo disso, só o eixo dominante vale.
        const val DIAG_RATIO = 0.5f
        // No stick as 8 direções dividem o círculo igualmente (fatias de 45°):
        // diagonal a partir de tan 22,5°.
        const val STICK_DIAG_RATIO = 0.414f
        // Zona morta do stick, em fração do raio da base.
        const val STICK_DEADZONE = 0.3f
        // Até onde (em raios da base) o dedo ainda conta como stick: dá pra
        // passar um pouco da borda sem soltar a direção.
        const val STICK_REACH = 1.6f
        const val STICK_DIRS = Btn.UP or Btn.DOWN or Btn.LEFT or Btn.RIGHT
    }

    var onMask: (Int) -> Unit = {}
    var onMenu: () -> Unit = {}

    /**
     * Layout dos controles. Paisagem: L/R/☰ no topo, direcional/A-B nos cantos de
     * baixo (tela cheia atrás). Retrato: tudo agrupado num cluster no rodapé
     * (estilo My Boy) — L/R/☰ no topo do cluster, direcional embaixo-esquerda, B/A
     * lado a lado embaixo-direita, Select/Start no centro acima do B/A.
     */
    var portrait = false
        set(value) { field = value; invalidate() }

    var model = PadModel.CLASSIC
        set(value) { field = value; invalidate() }

    /** Escala do tamanho dos controles (0,7–1,3); as distâncias às bordas escalam junto. */
    var scale = 1f
        set(value) { field = value.coerceIn(0.5f, 1.5f); invalidate() }

    /** Opacidade do preenchimento dos botões (0–1). */
    var opacity = 70 / 255f
        set(value) {
            field = value.coerceIn(0f, 1f)
            fill.alpha = (field * 255).toInt()
            invalidate()
        }

    /** Vibra um toque curto a cada botão novo pressionado. */
    var haptics = false

    /** Com controle físico: só o botão Menu fica (o resto some e não recebe toque). */
    var padHidden = false
        set(value) {
            field = value
            if (value && mask != 0) {
                mask = 0
                onMask(0)
            }
            invalidate()
        }

    private val d = resources.displayMetrics.density
    private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(70, 255, 255, 255) }
    private val active = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(140, 120, 200, 255) }
    private val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        color = Color.argb(70, 255, 255, 255)
    }
    private val text = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(200, 255, 255, 255)
        textAlign = Paint.Align.CENTER
        textSize = 16 * d
    }
    private val stickBase = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.WHITE }
    private val path = Path()
    private val rect = RectF()

    private var mask = 0
    // Deslocamento do dedo no stick (px, relativo ao centro), pro desenho do pino.
    private var knobX = 0f
    private var knobY = 0f

    private val vibrator: Vibrator? = context.getSystemService(Vibrator::class.java)

    // ── Geometria (recalculada conforme o tamanho/orientação/escala) ──────────
    // As posições são offsets a partir dos cantos, multiplicados pela escala:
    // botões maiores se afastam juntos da borda, sem se sobrepor.
    private val s get() = scale * d
    private val pad get() = 24 * d
    private val arm get() = (if (portrait) 56 else 64) * s // meio-comprimento do braço do D-pad
    private val thick get() = (if (portrait) 42 else 46) * s // largura do braço
    private val abR get() = (if (portrait) 42 else 44) * s
    private val seSize get() = 30 * s
    private val seHalfW get() = 44 * s
    private val lrW get() = (if (portrait) 110 else 120) * s
    private val lrH get() = 48 * s
    private val stickR get() = arm + thick / 2 // raio da base do stick = alcance do D-pad

    private val dpadCx get() = pad + (if (portrait) 62 else 96) * s
    private val dpadCy get() = height - (if (portrait) 120 * s else pad + 96 * s)
    // Retrato: A/B encostados mais à direita (folga pro stick, que é mais largo
    // que o D-pad).
    private val aCx get() = width - pad - (if (portrait) 48 else 64) * s
    private val aCy get() = height - (if (portrait) 105 * s else pad + 120 * s)
    private val bCx get() = width - pad - (if (portrait) 150 else 152) * s
    private val bCy get() = height - (if (portrait) 105 * s else pad + 64 * s)
    private val startCx get() = width / 2f + (if (portrait) 70 else 60) * s
    private val selCx get() = width / 2f - (if (portrait) 70 else 60) * s
    // Retrato: Select/Start ficam no respiro ENTRE a linha L/R/☰ e a fileira do
    // direcional/A-B (centro), bem acima do direcional pra não encavalar.
    private val seCy get() = height - (if (portrait) 250 * s else pad + 24 * d)

    // Linha L/R/☰: no topo da view (paisagem) ou no topo do cluster (retrato).
    private val rowTop get() = if (portrait) height - 345 * s else pad
    private fun lRect() = RectF(pad, rowTop, pad + lrW, rowTop + lrH)
    private fun rRect() = RectF(width - pad - lrW, rowTop, width - pad, rowTop + lrH)
    private fun menuRect() = RectF(width / 2f - 60 * s, rowTop, width / 2f + 60 * s, rowTop + lrH)

    /** Direções pra um toque em (dx, dy) do centro, com [ratio] de diagonal. */
    private fun directions(dx: Float, dy: Float, ratio: Float): Int {
        var m = 0
        val ax = kotlin.math.abs(dx)
        val ay = kotlin.math.abs(dy)
        if (ax >= ay * ratio) m = m or if (dx < 0) Btn.LEFT else Btn.RIGHT
        if (ay >= ax * ratio) m = m or if (dy < 0) Btn.UP else Btn.DOWN
        return m
    }

    /**
     * Botões pressionados por um ponteiro na posição (x, y). Os botões têm
     * prioridade: um toque que cai em A/B/Start/Select/L/R não vira direção —
     * o alcance do stick passa da borda da base e não pode roubar a beirada do B.
     */
    private fun hit(x: Float, y: Float): Int {
        if (padHidden) return 0
        var m = 0
        if (dist(x, y, aCx, aCy) <= abR) m = m or Btn.A
        if (dist(x, y, bCx, bCy) <= abR) m = m or Btn.B
        if (inPill(x, y, startCx, seCy)) m = m or Btn.START
        if (inPill(x, y, selCx, seCy)) m = m or Btn.SELECT
        if (lRect().contains(x, y)) m = m or Btn.L
        if (rRect().contains(x, y)) m = m or Btn.R
        if (m != 0) return m
        val dx = x - dpadCx
        val dy = y - dpadCy
        when (model) {
            // D-pad e setas: dentro da caixa do "+", a direção sai do eixo
            // DOMINANTE; a secundária só numa diagonal de verdade — encostar perto
            // da ponta de um braço não dispara a direção vizinha. Nas setas a
            // caixa cobre os cantos entre duas setas, então a diagonal continua
            // possível. Zona morta central pra não registrar toques no miolo.
            PadModel.CLASSIC, PadModel.ARROWS -> {
                val reach = arm + thick / 2f
                if (kotlin.math.abs(dx) <= reach && kotlin.math.abs(dy) <= reach &&
                    kotlin.math.hypot(dx.toDouble(), dy.toDouble()) >= 24 * s
                ) {
                    m = directions(dx, dy, DIAG_RATIO)
                }
            }
            PadModel.STICK -> {
                val dist = kotlin.math.hypot(dx.toDouble(), dy.toDouble()).toFloat()
                if (dist <= stickR * STICK_REACH && dist >= stickR * STICK_DEADZONE) {
                    m = directions(dx, dy, STICK_DIAG_RATIO)
                }
            }
        }
        return m
    }

    private fun dist(x: Float, y: Float, cx: Float, cy: Float) =
        kotlin.math.hypot((x - cx).toDouble(), (y - cy).toDouble()).toFloat()

    private fun inPill(x: Float, y: Float, cx: Float, cy: Float) =
        x >= cx - seHalfW && x <= cx + seHalfW && y >= cy - seSize && y <= cy + seSize

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
                var kx = 0f
                var ky = 0f
                if (event.actionMasked != MotionEvent.ACTION_UP &&
                    event.actionMasked != MotionEvent.ACTION_CANCEL
                ) {
                    val up = event.actionMasked == MotionEvent.ACTION_POINTER_UP
                    val skip = if (up) event.actionIndex else -1
                    for (i in 0 until event.pointerCount) {
                        if (i == skip) continue
                        val x = event.getX(i)
                        val y = event.getY(i)
                        val h = hit(x, y)
                        m = m or h
                        // Pino do stick acompanha o dedo que está nele (até a borda),
                        // a menos que o dedo esteja num botão (os botões têm prioridade).
                        if (model == PadModel.STICK && !padHidden && (h and STICK_DIRS.inv()) == 0 &&
                            dist(x, y, dpadCx, dpadCy) <= stickR * STICK_REACH
                        ) {
                            kx = x - dpadCx
                            ky = y - dpadCy
                        }
                    }
                }
                if (kx != knobX || ky != knobY) {
                    knobX = kx
                    knobY = ky
                    if (m == mask) invalidate()
                }
                if (m != mask) {
                    if (haptics && (m and mask.inv()) != 0) tick()
                    mask = m
                    onMask(m)
                    invalidate()
                }
            }
        }
        return true
    }

    /** Vibração curta de "clique" (efeito do sistema no Android 10+, senão 12 ms). */
    private fun tick() {
        val v = vibrator ?: return
        try {
            when {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q ->
                    v.vibrate(VibrationEffect.createPredefined(VibrationEffect.EFFECT_TICK))
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.O ->
                    v.vibrate(VibrationEffect.createOneShot(12, VibrationEffect.DEFAULT_AMPLITUDE))
                else -> @Suppress("DEPRECATION") v.vibrate(12)
            }
        } catch (e: Exception) {
            // Sem vibração disponível: segue sem.
        }
    }

    override fun onDraw(canvas: Canvas) {
        text.textSize = 16 * s
        val o = menuRect()
        rect(canvas, o.left, o.top, o.right, o.bottom, "☰ MENU", false)
        if (padHidden) return

        when (model) {
            PadModel.CLASSIC -> drawCross(canvas)
            PadModel.STICK -> drawStick(canvas)
            PadModel.ARROWS -> drawArrows(canvas)
        }
        circle(canvas, aCx, aCy, abR, "A", mask and Btn.A != 0)
        circle(canvas, bCx, bCy, abR, "B", mask and Btn.B != 0)
        text.textSize = 13 * s
        pill(canvas, startCx, seCy, "START", mask and Btn.START != 0)
        pill(canvas, selCx, seCy, "SEL", mask and Btn.SELECT != 0)
        text.textSize = 16 * s
        val l = lRect()
        rect(canvas, l.left, l.top, l.right, l.bottom, "L", mask and Btn.L != 0)
        val r = rRect()
        rect(canvas, r.left, r.top, r.right, r.bottom, "R", mask and Btn.R != 0)
    }

    /** D-pad em cruz, com a(s) direção(ões) ativa(s) realçada(s). */
    private fun drawCross(canvas: Canvas) {
        val cx = dpadCx
        val cy = dpadCy
        val a = arm
        val t = thick
        val rr = 10 * s
        canvas.drawRoundRect(RectF(cx - t / 2, cy - a, cx + t / 2, cy + a), rr, rr, fill)
        canvas.drawRoundRect(RectF(cx - a, cy - t / 2, cx + a, cy + t / 2), rr, rr, fill)
        if (mask and Btn.UP != 0) canvas.drawRoundRect(RectF(cx - t / 2, cy - a, cx + t / 2, cy), rr, rr, active)
        if (mask and Btn.DOWN != 0) canvas.drawRoundRect(RectF(cx - t / 2, cy, cx + t / 2, cy + a), rr, rr, active)
        if (mask and Btn.LEFT != 0) canvas.drawRoundRect(RectF(cx - a, cy - t / 2, cx, cy + t / 2), rr, rr, active)
        if (mask and Btn.RIGHT != 0) canvas.drawRoundRect(RectF(cx, cy - t / 2, cx + a, cy + t / 2), rr, rr, active)
    }

    /** Stick: base em anel, zona morta tracejada e o pino seguindo o dedo. */
    private fun drawStick(canvas: Canvas) {
        val cx = dpadCx
        val cy = dpadCy
        val r = stickR
        stickBase.alpha = (fill.alpha * 0.45f).toInt()
        canvas.drawCircle(cx, cy, r, stickBase)
        ring.strokeWidth = 2 * s
        ring.alpha = fill.alpha
        canvas.drawCircle(cx, cy, r, ring)
        // Pino: limitado à borda da base.
        var kx = knobX
        var ky = knobY
        val len = kotlin.math.hypot(kx.toDouble(), ky.toDouble()).toFloat()
        val maxLen = r - 30 * s
        if (len > maxLen && len > 0f) {
            kx *= maxLen / len
            ky *= maxLen / len
        }
        val dirs = mask and (Btn.UP or Btn.DOWN or Btn.LEFT or Btn.RIGHT)
        canvas.drawCircle(cx + kx, cy + ky, 30 * s, if (dirs != 0) active else fill)
    }

    /** Quatro setas separadas (pentágonos apontando pro centro), com glifo. */
    private fun drawArrows(canvas: Canvas) {
        val cx = dpadCx
        val cy = dpadCy
        val a = arm
        val t = thick
        val gap = 8 * s // meia-folga no centro entre as setas
        val tip = (a - gap) * 0.32f // comprimento da ponta
        fun arrow(dir: Int, on: Boolean) {
            path.reset()
            when (dir) {
                Btn.UP -> {
                    path.moveTo(cx - t / 2, cy - a); path.lineTo(cx + t / 2, cy - a)
                    path.lineTo(cx + t / 2, cy - gap - tip); path.lineTo(cx, cy - gap)
                    path.lineTo(cx - t / 2, cy - gap - tip)
                }
                Btn.DOWN -> {
                    path.moveTo(cx - t / 2, cy + a); path.lineTo(cx + t / 2, cy + a)
                    path.lineTo(cx + t / 2, cy + gap + tip); path.lineTo(cx, cy + gap)
                    path.lineTo(cx - t / 2, cy + gap + tip)
                }
                Btn.LEFT -> {
                    path.moveTo(cx - a, cy - t / 2); path.lineTo(cx - a, cy + t / 2)
                    path.lineTo(cx - gap - tip, cy + t / 2); path.lineTo(cx - gap, cy)
                    path.lineTo(cx - gap - tip, cy - t / 2)
                }
                else -> {
                    path.moveTo(cx + a, cy - t / 2); path.lineTo(cx + a, cy + t / 2)
                    path.lineTo(cx + gap + tip, cy + t / 2); path.lineTo(cx + gap, cy)
                    path.lineTo(cx + gap + tip, cy - t / 2)
                }
            }
            path.close()
            canvas.drawPath(path, if (on) active else fill)
            // Glifo: triângulo pequeno apontando pra fora, perto da ponta externa.
            val g = 6 * s
            val far = a - 14 * s
            path.reset()
            when (dir) {
                Btn.UP -> { path.moveTo(cx, cy - far - g); path.lineTo(cx + g, cy - far + g * 0.6f); path.lineTo(cx - g, cy - far + g * 0.6f) }
                Btn.DOWN -> { path.moveTo(cx, cy + far + g); path.lineTo(cx + g, cy + far - g * 0.6f); path.lineTo(cx - g, cy + far - g * 0.6f) }
                Btn.LEFT -> { path.moveTo(cx - far - g, cy); path.lineTo(cx - far + g * 0.6f, cy - g); path.lineTo(cx - far + g * 0.6f, cy + g) }
                else -> { path.moveTo(cx + far + g, cy); path.lineTo(cx + far - g * 0.6f, cy - g); path.lineTo(cx + far - g * 0.6f, cy + g) }
            }
            path.close()
            canvas.drawPath(path, text)
        }
        arrow(Btn.UP, mask and Btn.UP != 0)
        arrow(Btn.DOWN, mask and Btn.DOWN != 0)
        arrow(Btn.LEFT, mask and Btn.LEFT != 0)
        arrow(Btn.RIGHT, mask and Btn.RIGHT != 0)
    }

    private fun circle(canvas: Canvas, cx: Float, cy: Float, r: Float, label: String, on: Boolean) {
        canvas.drawCircle(cx, cy, r, if (on) active else fill)
        canvas.drawText(label, cx, cy + text.textSize * 0.37f, text)
    }

    private fun pill(canvas: Canvas, cx: Float, cy: Float, label: String, on: Boolean) {
        rect.set(cx - seHalfW, cy - seSize, cx + seHalfW, cy + seSize)
        canvas.drawRoundRect(rect, seSize, seSize, if (on) active else fill)
        canvas.drawText(label, cx, cy + text.textSize * 0.35f, text)
    }

    private fun rect(canvas: Canvas, l: Float, t: Float, r: Float, b: Float, label: String, on: Boolean) {
        rect.set(l, t, r, b)
        val rr = 10 * s
        canvas.drawRoundRect(rect, rr, rr, if (on) active else fill)
        canvas.drawText(label, (l + r) / 2, (t + b) / 2 + text.textSize * 0.37f, text)
    }
}
