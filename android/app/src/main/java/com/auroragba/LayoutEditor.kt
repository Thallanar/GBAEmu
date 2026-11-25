package com.auroragba

import android.app.Activity
import android.app.Dialog
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.drawable.ColorDrawable
import android.view.Gravity
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.SeekBar

/**
 * Editor de layout do gamepad: por cima do jogo pausado, uma cópia dos controles
 * em modo edição ocupa exatamente o lugar dos reais. Tocar escolhe a peça,
 * arrastar move, o slider muda o tamanho dela. Cada orientação tem o seu layout
 * (girar o aparelho no meio da edição passa a editar o da outra). Salvar devolve
 * os dois por [onSave]; Cancelar/voltar descarta.
 *
 * Retrato: barra no topo e o painel logo abaixo, sobre a tela do jogo (os
 * controles ficam presos à área deles, então nunca passam por baixo). Paisagem:
 * barra + painel num cartão central que dá pra arrastar pela barra.
 */
class LayoutEditor(
    private val act: Activity,
    private val host: Host,
    private val real: ControlsView,
    private val onSave: (portrait: PadLayout, landscape: PadLayout) -> Unit,
    private val onClose: () -> Unit,
) {
    private val ui = Ui(act)
    private fun t(id: Int) = host.tr(id)

    private val pad = ControlsView(act).apply {
        model = real.model
        scale = real.scale
        opacity = real.opacity
        layoutPortrait = real.layoutPortrait.copy()
        layoutLandscape = real.layoutLandscape.copy()
        editing = true
    }

    private val root = FrameLayout(act).apply { setBackgroundColor(0x8C000000.toInt()) }
    private val subtitle = ui.text("", 12f, Pal.MUTED).apply { gravity = Gravity.CENTER }
    private val partName = ui.text("", 15f, bold = true)
    private val sizeLabel = ui.text("", 14f, Pal.ACCENT, bold = true)
    private val slider = SeekBar(act).apply {
        max = (PadLayout.SIZE_MAX - PadLayout.SIZE_MIN) / STEP
        progressTintList = ColorStateList.valueOf(Pal.ACCENT)
        thumbTintList = ColorStateList.valueOf(Pal.ACCENT)
        progressBackgroundTintList = ColorStateList.valueOf(Pal.OFF)
        setOnSeekBarChangeListener(object : SeekBar.OnSeekBarChangeListener {
            override fun onProgressChanged(sb: SeekBar, p: Int, fromUser: Boolean) {
                if (!fromUser) return
                pad.layout[pad.selected].size = PadLayout.SIZE_MIN + p * STEP
                pad.invalidate()
                syncPanel()
            }
            override fun onStartTrackingTouch(sb: SeekBar) {}
            override fun onStopTrackingTouch(sb: SeekBar) {}
        })
    }
    private var chrome: LinearLayout? = null

    // Reposiciona a cópia quando os controles reais mudam de lugar (rotação).
    private val follow = View.OnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> root.post { place() } }

    private val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen).apply {
        window?.setBackgroundDrawable(ColorDrawable(Color.TRANSPARENT))
        window?.clearFlags(WindowManager.LayoutParams.FLAG_DIM_BEHIND)
        setOnKeyListener { _, keyCode, event ->
            if (keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) dismiss()
            keyCode == KeyEvent.KEYCODE_BACK
        }
        setOnDismissListener {
            real.removeOnLayoutChangeListener(follow)
            real.visibility = View.VISIBLE
            onClose()
        }
    }

    fun show() {
        pad.onSelect = { syncPanel() }
        root.addView(pad, FrameLayout.LayoutParams(0, 0))
        dialog.setContentView(root)
        // A cópia desenha no lugar dos reais; esconde os reais (sem tirar do layout).
        real.visibility = View.INVISIBLE
        real.addOnLayoutChangeListener(follow)
        dialog.show()
        root.post { place() }
    }

    /** Depois de uma rotação: realinha quando o layout novo assentar. */
    fun relayout() {
        root.post { place() }
    }

    /** Alinha a cópia com os controles reais e monta a barra/painel da orientação. */
    private fun place() {
        if (!dialog.isShowing) return
        val portrait = act.resources.configuration.orientation ==
            android.content.res.Configuration.ORIENTATION_PORTRAIT
        val r = IntArray(2).also { real.getLocationOnScreen(it) }
        val o = IntArray(2).also { root.getLocationOnScreen(it) }
        pad.layoutParams = FrameLayout.LayoutParams(real.width, real.height).apply {
            leftMargin = r[0] - o[0]
            topMargin = r[1] - o[1]
        }
        if (pad.portrait != portrait || chrome == null) {
            pad.portrait = portrait
            buildChrome(portrait)
        }
        syncPanel()
    }

    private fun buildChrome(portrait: Boolean) {
        chrome?.let { root.removeView(it) }
        // Textos e slider sobrevivem à troca de orientação: soltam do pai antigo.
        for (v in listOf(subtitle, partName, sizeLabel, slider)) (v.parent as? android.view.ViewGroup)?.removeView(v)
        val bar = ui.box(false, 8).apply {
            gravity = Gravity.CENTER_VERTICAL
            minimumHeight = ui.dp(56)
            setPadding(ui.dp(8), 0, ui.dp(8), 0)
            addView(ui.text(t(R.string.cancel), 15f, bold = true).apply {
                gravity = Gravity.CENTER
                minHeight = ui.dp(44)
                setPadding(ui.dp(12), 0, ui.dp(12), 0)
                background = ui.touchable(ui.rounded(0x00000000, 10))
                setOnClickListener { dialog.dismiss() }
            })
            addView(ui.box(true, 1).apply {
                gravity = Gravity.CENTER
                addView(ui.text(t(R.string.editor_title), 15f, bold = true).apply { gravity = Gravity.CENTER })
                addView(subtitle)
            }, ui.lp(0, Ui.WRAP, 1f))
            addView(ui.text(t(R.string.editor_save), 15f, Pal.ON_ACCENT, bold = true).apply {
                gravity = Gravity.CENTER
                minHeight = ui.dp(40)
                setPadding(ui.dp(16), 0, ui.dp(16), 0)
                background = ui.touchable(ui.rounded(Pal.ACCENT, 10))
                setOnClickListener {
                    onSave(pad.layoutPortrait, pad.layoutLandscape)
                    dialog.dismiss()
                }
            })
        }
        val panel = ui.box(true, 10).apply {
            setPadding(ui.dp(16), ui.dp(14), ui.dp(16), ui.dp(6))
            addView(ui.box(false).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(partName, ui.lp(0, Ui.WRAP, 1f))
                addView(sizeLabel)
            })
            addView(slider, ui.lp(Ui.MATCH, ui.dp(28)))
            addView(ui.box(false, 8).apply {
                gravity = Gravity.CENTER_VERTICAL
                addView(ui.text(t(R.string.editor_hint), 12f, Pal.MUTED), ui.lp(0, Ui.WRAP, 1f))
                addView(ui.text(t(R.string.editor_restore), 13f, Pal.ACCENT, bold = true).apply {
                    gravity = Gravity.CENTER
                    minHeight = ui.dp(44)
                    setPadding(ui.dp(4), 0, ui.dp(4), 0)
                    setOnClickListener {
                        pad.layout.reset()
                        pad.invalidate()
                        syncPanel()
                    }
                })
            })
        }
        val box = ui.box(true, 0)
        if (portrait) {
            bar.background = ColorDrawable(Pal.SURFACE)
            panel.background = ui.rounded(Pal.SURFACE, 16, 0xFF33333D.toInt())
            box.addView(bar, ui.lp())
            box.addView(panel, ui.lp().apply { setMargins(ui.dp(16), ui.dp(12), ui.dp(16), 0) })
            root.addView(box, FrameLayout.LayoutParams(Ui.MATCH, Ui.WRAP, Gravity.TOP))
        } else {
            box.background = ui.rounded(Pal.SURFACE, 16, 0xFF33333D.toInt())
            box.addView(bar, ui.lp())
            box.addView(android.view.View(act).apply { setBackgroundColor(Pal.DIVIDER) }, ui.lp(Ui.MATCH, 1))
            box.addView(panel, ui.lp())
            dragBy(bar, box)
            root.addView(box, FrameLayout.LayoutParams(ui.dp(380), Ui.WRAP, Gravity.CENTER))
        }
        chrome = box
    }

    /** Arrastar [handle] move [target] (o cartão da paisagem sai da frente). */
    private fun dragBy(handle: View, target: View) {
        var x0 = 0f
        var y0 = 0f
        handle.setOnTouchListener { _, e ->
            when (e.actionMasked) {
                MotionEvent.ACTION_DOWN -> { x0 = e.rawX - target.translationX; y0 = e.rawY - target.translationY }
                MotionEvent.ACTION_MOVE -> {
                    target.translationX = e.rawX - x0
                    target.translationY = e.rawY - y0
                }
            }
            true
        }
    }

    private fun syncPanel() {
        val p = pad.selected
        val size = pad.layout[p].size
        partName.text = t(partLabel(p))
        sizeLabel.text = "$size%"
        slider.progress = (size - PadLayout.SIZE_MIN) / STEP
        val orient = t(if (pad.portrait) R.string.editor_portrait else R.string.editor_landscape)
        val model = t(when (pad.model) {
            PadModel.CLASSIC -> R.string.pad_classic
            PadModel.STICK -> R.string.pad_stick
            PadModel.ARROWS -> R.string.pad_arrows
        })
        subtitle.text = "$orient · $model"
    }

    private fun partLabel(p: PadPart) = when (p) {
        PadPart.L -> R.string.part_l
        PadPart.MENU -> R.string.part_menu
        PadPart.R -> R.string.part_r
        PadPart.SELECT -> R.string.part_select
        PadPart.START -> R.string.part_start
        PadPart.DPAD -> R.string.part_dpad
        PadPart.B -> R.string.part_b
        PadPart.A -> R.string.part_a
    }

    private companion object {
        const val STEP = 5
    }
}
