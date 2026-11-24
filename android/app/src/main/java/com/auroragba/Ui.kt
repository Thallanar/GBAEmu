package com.auroragba

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Typeface
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.SeekBar
import android.widget.TextView

/** Paleta do app (a mesma do canvas de design). */
object Pal {
    const val BG = 0xFF121216.toInt()
    const val SURFACE = 0xFF1C1C22.toInt()
    const val SURFACE2 = 0xFF26262E.toInt()
    const val BORDER = 0xFF3A3A45.toInt()
    const val DIVIDER = 0xFF2C2C35.toInt()
    const val TEXT = 0xFFF2F2F5.toInt()
    const val MUTED = 0xFFA3A3AD.toInt()
    const val FAINT = 0xFF8C8C96.toInt()
    const val ACCENT = 0xFF78C8FF.toInt()
    const val ON_ACCENT = 0xFF0B2233.toInt()
    const val ACCENT_SOFT = 0x2478C8FF
    const val OFF = 0xFF4A4A55.toInt()
}

/**
 * Mini-kit das telas do app (menu e configurações), montadas em código como o
 * resto do app. Concentra as formas do design — cartão, linha com seta, switch,
 * slider, seletor segmentado — pra as telas ficarem consistentes.
 */
class Ui(val ctx: Context) {
    private val density = ctx.resources.displayMetrics.density

    fun dp(v: Int) = (v * density).toInt()

    fun rounded(color: Int, radiusDp: Int, stroke: Int? = null, strokeDp: Int = 1): GradientDrawable =
        GradientDrawable().apply {
            setColor(color)
            cornerRadius = radiusDp * density
            if (stroke != null) setStroke(dp(strokeDp), stroke)
        }

    /** [bg] com o efeito de toque (ripple) por cima, recortado pela própria forma. */
    fun touchable(bg: Drawable): Drawable =
        RippleDrawable(ColorStateList.valueOf(0x33FFFFFF), bg, null)

    fun lp(w: Int = MATCH, h: Int = WRAP, weight: Float = 0f) =
        LinearLayout.LayoutParams(w, h, weight)

    /** Coluna/linha com espaçamento [gapDp] entre os filhos. */
    fun box(vertical: Boolean, gapDp: Int = 0): LinearLayout = LinearLayout(ctx).apply {
        orientation = if (vertical) LinearLayout.VERTICAL else LinearLayout.HORIZONTAL
        if (gapDp > 0) {
            dividerDrawable = GradientDrawable().apply {
                if (vertical) setSize(0, dp(gapDp)) else setSize(dp(gapDp), 0)
            }
            showDividers = LinearLayout.SHOW_DIVIDER_MIDDLE
        }
    }

    fun text(s: CharSequence, sizeSp: Float = 15f, color: Int = Pal.TEXT, bold: Boolean = false) =
        TextView(ctx).apply {
            text = s
            textSize = sizeSp
            setTextColor(color)
            if (bold) typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
            setLineSpacing(0f, 1.15f)
        }

    /** Rótulo de seção em caixa alta (ex.: "VÍDEO"). */
    fun sectionLabel(s: String) = text(s.uppercase(), 12f, Pal.MUTED, bold = true).apply {
        letterSpacing = 0.08f
        setPadding(dp(4), 0, dp(4), 0)
    }

    fun icon(res: Int, tint: Int = Pal.ACCENT, sizeDp: Int = 22) = ImageView(ctx).apply {
        setImageResource(res)
        imageTintList = ColorStateList.valueOf(tint)
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp))
    }

    /** Ícone dentro do quadrado arredondado das linhas de configuração. */
    fun iconTile(res: Int) = FrameLayout(ctx).apply {
        background = rounded(Pal.SURFACE2, 12)
        layoutParams = LinearLayout.LayoutParams(dp(40), dp(40))
        addView(icon(res).apply {
            layoutParams = FrameLayout.LayoutParams(dp(22), dp(22), Gravity.CENTER)
        })
    }

    /** Cartão (superfície arredondada) com conteúdo em coluna. */
    fun card(gapDp: Int = 12, padDp: Int = 16) = box(true, gapDp).apply {
        background = rounded(Pal.SURFACE, 16)
        setPadding(dp(padDp), dp(padDp), dp(padDp), dp(padDp))
    }

    /** Título + subtítulo empilhados (subtítulo opcional). */
    fun titled(title: String, sub: String?) = box(true, 3).apply {
        addView(text(title, 16f, bold = true))
        if (!sub.isNullOrEmpty()) addView(text(sub, 13f, Pal.MUTED))
    }

    /** Linha de navegação: ícone, título/subtítulo e seta. */
    fun navRow(iconRes: Int?, title: String, sub: String?, onClick: () -> Unit) = box(false, 14).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = touchable(rounded(Pal.SURFACE, 16))
        minimumHeight = dp(72)
        setPadding(dp(16), dp(14), dp(16), dp(14))
        if (iconRes != null) addView(iconTile(iconRes))
        addView(titled(title, sub), lp(0, WRAP, 1f))
        addView(icon(R.drawable.ic_chevron, Pal.MUTED, 18))
        isClickable = true
        setOnClickListener { onClick() }
    }

    /** Linha simples dentro de um cartão agrupado: título à esquerda, valor + seta à direita. */
    fun valueRow(title: String, value: String?, onClick: () -> Unit) = box(false, 8).apply {
        gravity = Gravity.CENTER_VERTICAL
        background = touchable(rounded(0x00000000, 0))
        minimumHeight = dp(60)
        setPadding(dp(16), 0, dp(16), 0)
        addView(text(title, 16f, bold = true), lp(0, WRAP, 1f))
        if (value != null) addView(text(value, 14f, Pal.MUTED))
        addView(icon(R.drawable.ic_chevron, Pal.MUTED, 18))
        isClickable = true
        setOnClickListener { onClick() }
    }

    /** Cartão que agrupa várias [valueRow] separadas por uma linha fina. */
    fun group(vararg rows: View) = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(Pal.SURFACE, 16)
        clipToOutline = true
        rows.forEachIndexed { i, row ->
            if (i > 0) addView(View(ctx).apply { setBackgroundColor(Pal.DIVIDER) },
                lp(MATCH, dp(1)).apply { setMargins(dp(16), 0, dp(16), 0) })
            addView(row)
        }
    }

    /**
     * Switch no visual do app (azul ligado, cinza desligado). Desenhado aqui: o
     * `Switch` do Android não tem aparência no tema antigo do app (`Theme.Black`).
     */
    fun switch(checked: Boolean, onChange: (Boolean) -> Unit) = Toggle(this, checked, onChange)

    /** Cartão com título/subtítulo e um switch à direita. */
    fun switchRow(title: String, sub: String?, checked: Boolean, onChange: (Boolean) -> Unit) =
        box(false, 12).apply {
            gravity = Gravity.CENTER_VERTICAL
            background = rounded(Pal.SURFACE, 16)
            setPadding(dp(16), dp(14), dp(16), dp(14))
            addView(titled(title, sub), lp(0, WRAP, 1f))
            val sw = switch(checked, onChange)
            addView(sw)
            setOnClickListener { sw.toggle() }
        }

    /** Cartão com título, valor atual ("80%") e um slider. */
    fun sliderRow(title: String, value: Int, min: Int, max: Int, step: Int, unit: String, onChange: (Int) -> Unit) =
        card(10).apply {
            val label = text("$value$unit", 15f, Pal.ACCENT, bold = true)
            addView(box(false).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(text(title, 16f, bold = true), lp(0, WRAP, 1f))
                addView(label)
            })
            addView(SeekBar(ctx).apply {
                this.max = (max - min) / step
                progress = (value - min) / step
                progressTintList = ColorStateList.valueOf(Pal.ACCENT)
                thumbTintList = ColorStateList.valueOf(Pal.ACCENT)
                progressBackgroundTintList = ColorStateList.valueOf(Pal.OFF)
                setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
                    override fun onProgressChanged(sb: SeekBar, p: Int, fromUser: Boolean) {
                        val v = min + p * step
                        label.text = "$v$unit"
                        if (fromUser) onChange(v)
                    }
                    override fun onStartTrackingTouch(sb: SeekBar) {}
                    override fun onStopTrackingTouch(sb: SeekBar) {}
                })
            }, lp(MATCH, dp(36)))
        }

    /** Seletor segmentado (uma opção ativa). Chama [onPick] com o índice escolhido. */
    fun segmented(options: List<String>, selected: Int, onPick: (Int) -> Unit): LinearLayout {
        val row = box(false, 6).apply {
            background = rounded(Pal.BG, 12)
            setPadding(dp(4), dp(4), dp(4), dp(4))
        }
        val buttons = options.map { opt -> text(opt, 14f, Pal.MUTED, bold = true) }
        fun paint(sel: Int) = buttons.forEachIndexed { i, b ->
            b.background = rounded(if (i == sel) Pal.SURFACE2 else 0x00000000, 9)
            b.setTextColor(if (i == sel) Pal.TEXT else Pal.MUTED)
        }
        buttons.forEachIndexed { i, b ->
            b.gravity = Gravity.CENTER
            b.minHeight = dp(44)
            b.setOnClickListener { paint(i); onPick(i) }
            row.addView(b, lp(0, WRAP, 1f))
        }
        paint(selected)
        return row
    }

    /** Botão cheio na cor de destaque. */
    fun accentButton(label: String, onClick: () -> Unit) = text(label, 15f, Pal.ON_ACCENT, bold = true).apply {
        gravity = Gravity.CENTER
        minHeight = dp(48)
        background = touchable(rounded(Pal.ACCENT, 12))
        setOnClickListener { onClick() }
    }

    /** Botão só com contorno. */
    fun outlineButton(label: String, onClick: () -> Unit) = text(label, 14f, Pal.TEXT, bold = true).apply {
        gravity = Gravity.CENTER
        minHeight = dp(48)
        background = touchable(rounded(0x00000000, 12, Pal.BORDER))
        setOnClickListener { onClick() }
    }

    companion object {
        const val MATCH = ViewGroup.LayoutParams.MATCH_PARENT
        const val WRAP = ViewGroup.LayoutParams.WRAP_CONTENT
    }
}

/** Switch próprio (pílula 52×32 com pino), ver [Ui.switch]. */
class Toggle(private val ui: Ui, checked: Boolean, private val onChange: (Boolean) -> Unit) :
    FrameLayout(ui.ctx) {
    private val knob = View(ui.ctx)

    var isChecked = checked
        private set

    init {
        addView(knob, LayoutParams(ui.dp(26), ui.dp(26)))
        layoutParams = LinearLayout.LayoutParams(ui.dp(52), ui.dp(32))
        isClickable = true
        isFocusable = true
        setOnClickListener { toggle() }
        paint()
    }

    fun toggle() {
        isChecked = !isChecked
        paint()
        onChange(isChecked)
    }

    private fun paint() {
        background = ui.rounded(if (isChecked) Pal.ACCENT else Pal.OFF, 16)
        knob.background = ui.rounded(if (isChecked) Pal.ON_ACCENT else Pal.TEXT, 13)
        knob.layoutParams = (knob.layoutParams as LayoutParams).apply {
            gravity = (if (isChecked) Gravity.END else Gravity.START) or Gravity.CENTER_VERTICAL
            setMargins(ui.dp(3), 0, ui.dp(3), 0)
        }
        contentDescription = null
        // Leitor de tela: anuncia como switch ligado/desligado.
        accessibilityDelegate = object : AccessibilityDelegate() {
            override fun onInitializeAccessibilityNodeInfo(host: View, info: android.view.accessibility.AccessibilityNodeInfo) {
                super.onInitializeAccessibilityNodeInfo(host, info)
                info.className = "android.widget.Switch"
                info.isCheckable = true
                info.isChecked = isChecked
            }
        }
    }
}

/** FrameLayout com altura = largura × [ratio] (ex.: 2/3 pra miniatura 3:2 do GBA). */
class AspectFrame(ctx: Context, private val ratio: Float) : FrameLayout(ctx) {
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val w = MeasureSpec.getSize(widthMeasureSpec)
        val h = (w * ratio).toInt()
        super.onMeasure(
            MeasureSpec.makeMeasureSpec(w, MeasureSpec.EXACTLY),
            MeasureSpec.makeMeasureSpec(h, MeasureSpec.EXACTLY),
        )
    }
}
