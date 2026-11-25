package com.auroragba

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.DashPathEffect
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

/** Peças do gamepad na tela que o editor de layout move e redimensiona. */
enum class PadPart(val key: String) {
    L("l"), MENU("menu"), R("r"), SELECT("sel"), START("start"), DPAD("dpad"), B("b"), A("a");

    companion object {
        fun from(key: String) = values().firstOrNull { it.key == key }
    }
}

/** Ajuste de uma peça: deslocamento (dp) a partir da posição padrão e tamanho (%). */
class PadTweak(var dx: Float = 0f, var dy: Float = 0f, var size: Int = 100) {
    val isDefault get() = dx == 0f && dy == 0f && size == 100
}

/**
 * Layout personalizado de uma orientação: um [PadTweak] por peça (todos zerados =
 * o layout padrão). O tamanho por peça multiplica o "Tamanho dos botões" geral.
 * Persistido como `chave:dx,dy,tamanho;…`, só com as peças alteradas.
 */
class PadLayout {
    private val tweaks = PadPart.values().map { PadTweak() }

    operator fun get(p: PadPart) = tweaks[p.ordinal]

    fun copy() = PadLayout().also { c ->
        PadPart.values().forEach { p -> c[p].apply { dx = this@PadLayout[p].dx; dy = this@PadLayout[p].dy; size = this@PadLayout[p].size } }
    }

    fun reset() = tweaks.forEach { it.dx = 0f; it.dy = 0f; it.size = 100 }

    fun encode(): String = PadPart.values().filter { !this[it].isDefault }.joinToString(";") { p ->
        val t = this[p]
        "${p.key}:${Math.round(t.dx)},${Math.round(t.dy)},${t.size}"
    }

    companion object {
        const val SIZE_MIN = 60
        const val SIZE_MAX = 160

        fun decode(s: String?): PadLayout {
            val l = PadLayout()
            s?.split(';')?.forEach { entry ->
                val kv = entry.split(':')
                val part = PadPart.from(kv.getOrNull(0) ?: return@forEach) ?: return@forEach
                val v = kv.getOrNull(1)?.split(',') ?: return@forEach
                l[part].dx = v.getOrNull(0)?.toFloatOrNull() ?: 0f
                l[part].dy = v.getOrNull(1)?.toFloatOrNull() ?: 0f
                l[part].size = (v.getOrNull(2)?.toIntOrNull() ?: 100).coerceIn(SIZE_MIN, SIZE_MAX)
            }
            return l
        }
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
        // Ordem de teste do toque no editor: botões por cima do direcional.
        val PICK_ORDER = listOf(PadPart.A, PadPart.B, PadPart.START, PadPart.SELECT,
            PadPart.L, PadPart.R, PadPart.MENU, PadPart.DPAD)
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

    /** Ajustes do editor de layout, um por orientação. */
    var layoutPortrait = PadLayout()
        set(value) { field = value; invalidate() }
    var layoutLandscape = PadLayout()
        set(value) { field = value; invalidate() }

    /** O layout da orientação atual. */
    val layout: PadLayout get() = if (portrait) layoutPortrait else layoutLandscape

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

    /**
     * Modo editor de layout: não gera input; tocar escolhe a peça ([selected],
     * avisada em [onSelect]) e arrastar a move. Desenha a grade e os contornos.
     */
    var editing = false
        set(value) { field = value; invalidate() }
    var selected = PadPart.A
        set(value) { field = value; invalidate() }
    var onSelect: (PadPart) -> Unit = {}

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
    // Editor: grade, contorno tracejado de cada peça e a peça escolhida em destaque.
    private val grid = Paint().apply { color = Color.argb(13, 255, 255, 255); strokeWidth = 1f }
    private val outline = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        color = Color.argb(115, 255, 255, 255)
        strokeWidth = 2 * d
        pathEffect = DashPathEffect(floatArrayOf(6 * d, 4 * d), 0f)
    }
    private val selStroke = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE
        color = Pal.ACCENT
        strokeWidth = 2 * d
    }
    private val selFill = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = Color.argb(77, 120, 200, 255) }
    private val path = Path()
    private val rect = RectF()

    private var mask = 0
    // Deslocamento do dedo no stick (px, relativo ao centro), pro desenho do pino.
    private var knobX = 0f
    private var knobY = 0f

    // Arrasto no editor: ponteiro, ponto inicial e deslocamento inicial (dp).
    private var dragId = -1
    private var dragX0 = 0f
    private var dragY0 = 0f
    private var offX0 = 0f
    private var offY0 = 0f

    private val vibrator: Vibrator? = context.getSystemService(Vibrator::class.java)

    // ── Geometria (recalculada a cada toque/desenho) ──────────────────────────
    // As posições padrão são offsets a partir dos cantos, multiplicados pela
    // escala geral: botões maiores se afastam juntos da borda, sem se sobrepor.
    // Por cima vêm os ajustes do editor (deslocamento + tamanho de cada peça),
    // com o centro preso dentro da view.
    private val s get() = scale * d
    private val pad get() = 24 * d
    private val n = PadPart.values().size
    private val defX = FloatArray(n)
    private val defY = FloatArray(n)
    private val cx = FloatArray(n)
    private val cy = FloatArray(n)
    private val hw = FloatArray(n) // meia-largura
    private val hh = FloatArray(n) // meia-altura

    /** Escala de uma peça: geral × tamanho próprio do editor. */
    private fun ps(p: PadPart) = s * layout[p].size / 100f

    // Direcional (dependem do tamanho da peça).
    private val kD get() = ps(PadPart.DPAD)
    private val arm get() = (if (portrait) 56 else 64) * kD // meio-comprimento do braço do D-pad
    private val thick get() = (if (portrait) 42 else 46) * kD // largura do braço
    private val stickR get() = arm + thick / 2 // raio da base do stick = alcance do D-pad

    private fun computeGeometry() {
        val p = portrait
        val w = width.toFloat()
        val h = height.toFloat()
        fun def(part: PadPart, x: Float, y: Float) {
            defX[part.ordinal] = x
            defY[part.ordinal] = y
        }
        // Retrato: A/B encostados mais à direita (folga pro stick, que é mais
        // largo que o D-pad); Select/Start no respiro ENTRE a linha L/R/☰ e a
        // fileira do direcional/A-B, bem acima do direcional pra não encavalar.
        def(PadPart.DPAD, pad + (if (p) 62 else 96) * s, h - (if (p) 120 * s else pad + 96 * s))
        def(PadPart.A, w - pad - (if (p) 48 else 64) * s, h - (if (p) 105 * s else pad + 120 * s))
        def(PadPart.B, w - pad - (if (p) 150 else 152) * s, h - (if (p) 105 * s else pad + 64 * s))
        val seCy = h - (if (p) 250 * s else pad + 24 * d)
        def(PadPart.START, w / 2 + (if (p) 70 else 60) * s, seCy)
        def(PadPart.SELECT, w / 2 - (if (p) 70 else 60) * s, seCy)
        // Linha L/R/☰: no topo da view (paisagem) ou no topo do cluster (retrato).
        val rowCy = (if (p) h - 345 * s else pad) + 24 * s
        val lrHalf = (if (p) 55 else 60) * s
        def(PadPart.L, pad + lrHalf, rowCy)
        def(PadPart.R, w - pad - lrHalf, rowCy)
        def(PadPart.MENU, w / 2, rowCy)

        for (part in PadPart.values()) {
            val i = part.ordinal
            val k = ps(part)
            when (part) {
                PadPart.DPAD -> { hw[i] = stickR; hh[i] = stickR }
                PadPart.A, PadPart.B -> { hw[i] = (if (p) 42 else 44) * k; hh[i] = hw[i] }
                PadPart.SELECT, PadPart.START -> { hw[i] = 44 * k; hh[i] = 30 * k }
                PadPart.L, PadPart.R -> { hw[i] = (if (p) 55 else 60) * k; hh[i] = 24 * k }
                PadPart.MENU -> { hw[i] = 60 * k; hh[i] = 24 * k }
            }
            val t = layout[part]
            cx[i] = clampIn(defX[i] + t.dx * d, hw[i], w)
            cy[i] = clampIn(defY[i] + t.dy * d, hh[i], h)
        }
    }

    /** Prende o centro de modo que a peça (meia-medida [half]) caiba em [0, size]. */
    private fun clampIn(v: Float, half: Float, size: Float) =
        if (size <= 2 * half) size / 2 else v.coerceIn(half, size - half)

    private fun inBox(p: PadPart, x: Float, y: Float) =
        kotlin.math.abs(x - cx[p.ordinal]) <= hw[p.ordinal] && kotlin.math.abs(y - cy[p.ordinal]) <= hh[p.ordinal]

    private fun inCircle(p: PadPart, x: Float, y: Float) =
        dist(x, y, cx[p.ordinal], cy[p.ordinal]) <= hw[p.ordinal]

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
        if (inCircle(PadPart.A, x, y)) m = m or Btn.A
        if (inCircle(PadPart.B, x, y)) m = m or Btn.B
        if (inBox(PadPart.START, x, y)) m = m or Btn.START
        if (inBox(PadPart.SELECT, x, y)) m = m or Btn.SELECT
        if (inBox(PadPart.L, x, y)) m = m or Btn.L
        if (inBox(PadPart.R, x, y)) m = m or Btn.R
        if (m != 0) return m
        val dx = x - cx[PadPart.DPAD.ordinal]
        val dy = y - cy[PadPart.DPAD.ordinal]
        when (model) {
            // D-pad e setas: dentro da caixa do "+", a direção sai do eixo
            // DOMINANTE; a secundária só numa diagonal de verdade — encostar perto
            // da ponta de um braço não dispara a direção vizinha. Nas setas a
            // caixa cobre os cantos entre duas setas, então a diagonal continua
            // possível. Zona morta central pra não registrar toques no miolo.
            PadModel.CLASSIC, PadModel.ARROWS -> {
                val reach = arm + thick / 2f
                if (kotlin.math.abs(dx) <= reach && kotlin.math.abs(dy) <= reach &&
                    kotlin.math.hypot(dx.toDouble(), dy.toDouble()) >= 24 * kD
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

    override fun onTouchEvent(event: MotionEvent): Boolean {
        computeGeometry()
        if (editing) return editTouch(event)
        // Toque novo no botão de menu (topo/centro) abre o menu e não vira input.
        if (event.actionMasked == MotionEvent.ACTION_DOWN ||
            event.actionMasked == MotionEvent.ACTION_POINTER_DOWN
        ) {
            val i = event.actionIndex
            if (inBox(PadPart.MENU, event.getX(i), event.getY(i))) {
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
                val dcx = cx[PadPart.DPAD.ordinal]
                val dcy = cy[PadPart.DPAD.ordinal]
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
                            dist(x, y, dcx, dcy) <= stickR * STICK_REACH
                        ) {
                            kx = x - dcx
                            ky = y - dcy
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

    /** Editor: tocar escolhe a peça; arrastar (com o primeiro dedo) a move. */
    private fun editTouch(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val x = event.x
                val y = event.y
                val part = PICK_ORDER.firstOrNull { inBox(it, x, y) } ?: return true
                if (part != selected) {
                    selected = part
                    onSelect(part)
                }
                dragId = event.getPointerId(0)
                dragX0 = x
                dragY0 = y
                offX0 = layout[part].dx
                offY0 = layout[part].dy
            }
            MotionEvent.ACTION_MOVE -> {
                val i = event.findPointerIndex(dragId)
                if (dragId < 0 || i < 0) return true
                val t = layout[selected]
                t.dx = offX0 + (event.getX(i) - dragX0) / d
                t.dy = offY0 + (event.getY(i) - dragY0) / d
                // Guarda o deslocamento já preso à tela, senão arrastar além da
                // borda acumularia um excesso invisível.
                computeGeometry()
                val k = selected.ordinal
                t.dx = (cx[k] - defX[k]) / d
                t.dy = (cy[k] - defY[k]) / d
                invalidate()
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> dragId = -1
            MotionEvent.ACTION_POINTER_UP ->
                if (event.getPointerId(event.actionIndex) == dragId) dragId = -1
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
        computeGeometry()
        if (editing) drawGrid(canvas)
        rectPart(canvas, PadPart.MENU, "☰ MENU", false)
        if (padHidden && !editing) return

        when (model) {
            PadModel.CLASSIC -> drawCross(canvas)
            PadModel.STICK -> drawStick(canvas)
            PadModel.ARROWS -> drawArrows(canvas)
        }
        circle(canvas, PadPart.A, "A", mask and Btn.A != 0)
        circle(canvas, PadPart.B, "B", mask and Btn.B != 0)
        pill(canvas, PadPart.START, "START", mask and Btn.START != 0)
        pill(canvas, PadPart.SELECT, "SEL", mask and Btn.SELECT != 0)
        rectPart(canvas, PadPart.L, "L", mask and Btn.L != 0)
        rectPart(canvas, PadPart.R, "R", mask and Btn.R != 0)
        if (editing) drawOutlines(canvas)
    }

    private fun drawGrid(canvas: Canvas) {
        val step = 16 * d
        var x = step
        while (x < width) { canvas.drawLine(x, 0f, x, height.toFloat(), grid); x += step }
        var y = step
        while (y < height) { canvas.drawLine(0f, y, width.toFloat(), y, grid); y += step }
    }

    /** Contorno tracejado de cada peça; a escolhida ganha realce e borda cheia. */
    private fun drawOutlines(canvas: Canvas) {
        for (p in PadPart.values()) {
            val i = p.ordinal
            val on = p == selected
            when (p) {
                PadPart.A, PadPart.B -> {
                    if (on) canvas.drawCircle(cx[i], cy[i], hw[i], selFill)
                    canvas.drawCircle(cx[i], cy[i], hw[i], if (on) selStroke else outline)
                }
                else -> {
                    rect.set(cx[i] - hw[i], cy[i] - hh[i], cx[i] + hw[i], cy[i] + hh[i])
                    val r = when (p) {
                        PadPart.SELECT, PadPart.START -> hh[i]
                        PadPart.DPAD -> 16 * ps(p)
                        else -> 10 * ps(p)
                    }
                    if (on) canvas.drawRoundRect(rect, r, r, selFill)
                    canvas.drawRoundRect(rect, r, r, if (on) selStroke else outline)
                }
            }
        }
    }

    /** D-pad em cruz, com a(s) direção(ões) ativa(s) realçada(s). */
    private fun drawCross(canvas: Canvas) {
        val cx = cx[PadPart.DPAD.ordinal]
        val cy = cy[PadPart.DPAD.ordinal]
        val a = arm
        val t = thick
        val rr = 10 * kD
        canvas.drawRoundRect(RectF(cx - t / 2, cy - a, cx + t / 2, cy + a), rr, rr, fill)
        canvas.drawRoundRect(RectF(cx - a, cy - t / 2, cx + a, cy + t / 2), rr, rr, fill)
        if (mask and Btn.UP != 0) canvas.drawRoundRect(RectF(cx - t / 2, cy - a, cx + t / 2, cy), rr, rr, active)
        if (mask and Btn.DOWN != 0) canvas.drawRoundRect(RectF(cx - t / 2, cy, cx + t / 2, cy + a), rr, rr, active)
        if (mask and Btn.LEFT != 0) canvas.drawRoundRect(RectF(cx - a, cy - t / 2, cx, cy + t / 2), rr, rr, active)
        if (mask and Btn.RIGHT != 0) canvas.drawRoundRect(RectF(cx, cy - t / 2, cx + a, cy + t / 2), rr, rr, active)
    }

    /** Stick: base em anel, zona morta tracejada e o pino seguindo o dedo. */
    private fun drawStick(canvas: Canvas) {
        val cx = cx[PadPart.DPAD.ordinal]
        val cy = cy[PadPart.DPAD.ordinal]
        val r = stickR
        val k = kD
        stickBase.alpha = (fill.alpha * 0.45f).toInt()
        canvas.drawCircle(cx, cy, r, stickBase)
        ring.strokeWidth = 2 * k
        ring.alpha = fill.alpha
        canvas.drawCircle(cx, cy, r, ring)
        // Pino: limitado à borda da base.
        var kx = knobX
        var ky = knobY
        val len = kotlin.math.hypot(kx.toDouble(), ky.toDouble()).toFloat()
        val maxLen = r - 30 * k
        if (len > maxLen && len > 0f) {
            kx *= maxLen / len
            ky *= maxLen / len
        }
        val dirs = mask and (Btn.UP or Btn.DOWN or Btn.LEFT or Btn.RIGHT)
        canvas.drawCircle(cx + kx, cy + ky, 30 * k, if (dirs != 0) active else fill)
    }

    /** Quatro setas separadas (pentágonos apontando pro centro), com glifo. */
    private fun drawArrows(canvas: Canvas) {
        val cx = cx[PadPart.DPAD.ordinal]
        val cy = cy[PadPart.DPAD.ordinal]
        val a = arm
        val t = thick
        val k = kD
        val gap = 8 * k // meia-folga no centro entre as setas
        val tip = (a - gap) * 0.32f // comprimento da ponta
        text.textSize = 16 * k
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
            val g = 6 * k
            val far = a - 14 * k
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

    private fun circle(canvas: Canvas, p: PadPart, label: String, on: Boolean) {
        val i = p.ordinal
        text.textSize = 16 * ps(p)
        canvas.drawCircle(cx[i], cy[i], hw[i], if (on) active else fill)
        canvas.drawText(label, cx[i], cy[i] + text.textSize * 0.37f, text)
    }

    private fun pill(canvas: Canvas, p: PadPart, label: String, on: Boolean) {
        val i = p.ordinal
        text.textSize = 13 * ps(p)
        rect.set(cx[i] - hw[i], cy[i] - hh[i], cx[i] + hw[i], cy[i] + hh[i])
        canvas.drawRoundRect(rect, hh[i], hh[i], if (on) active else fill)
        canvas.drawText(label, cx[i], cy[i] + text.textSize * 0.35f, text)
    }

    private fun rectPart(canvas: Canvas, p: PadPart, label: String, on: Boolean) {
        val i = p.ordinal
        text.textSize = 16 * ps(p)
        rect.set(cx[i] - hw[i], cy[i] - hh[i], cx[i] + hw[i], cy[i] + hh[i])
        val rr = 10 * ps(p)
        canvas.drawRoundRect(rect, rr, rr, if (on) active else fill)
        canvas.drawText(label, cx[i], cy[i] + text.textSize * 0.37f, text)
    }
}
