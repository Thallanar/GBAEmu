package com.auroragba

import android.app.Activity
import android.app.Dialog
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.widget.FrameLayout
import android.widget.GridLayout
import android.widget.LinearLayout
import android.widget.ScrollView

/** Um alvo do Shiny Hunter como a tela mostra: nome traduzido e grupo. */
class HuntTarget(val name: String, val kind: Int) {
    companion object {
        // Mesmos códigos do JNI (`huntTargetKind`).
        const val LEGENDARY = 0
        const val STARTER = 1
        const val GIFT = 2
        const val WILD = 3
    }
}

/** Tela cheia com cabeçalho (seta + título/subtítulo) e corpo rolável; base das telas do Hunter. */
private abstract class HunterDialog(protected val act: Activity, protected val host: Host) {
    protected val ui = Ui(act)
    protected var closingToMenu = false

    protected val dialog = Dialog(act, android.R.style.Theme_Black_NoTitleBar_Fullscreen).apply {
        setOnKeyListener { _, keyCode, event ->
            if (keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_UP) back()
            keyCode == KeyEvent.KEYCODE_BACK
        }
    }

    protected fun t(id: Int, vararg args: Any) = host.tr(id, *args)

    protected open fun back() {
        closingToMenu = true
        dialog.dismiss()
    }

    /** Monta cabeçalho + [body] (rolável) + [footer] fixo embaixo. */
    protected fun show(title: String, subtitle: String?, body: View, footer: View?) {
        val backButton = FrameLayout(act).apply {
            background = ui.touchable(ui.rounded(0x00000000, 22))
            contentDescription = t(R.string.back)
            addView(ui.icon(R.drawable.ic_back, Pal.TEXT, 22).apply {
                layoutParams = FrameLayout.LayoutParams(ui.dp(22), ui.dp(22), Gravity.CENTER)
            })
            setOnClickListener { back() }
        }
        val header = ui.box(false, 8).apply {
            gravity = Gravity.CENTER_VERTICAL
            setPadding(ui.dp(12), ui.dp(16), ui.dp(12), ui.dp(8))
            addView(backButton, LinearLayout.LayoutParams(ui.dp(44), ui.dp(44)))
            addView(ui.box(true, 1).apply {
                addView(ui.text(title, 22f, bold = true))
                if (subtitle != null) addView(ui.text(subtitle, 13f, Pal.MUTED))
            })
        }
        val root = ui.box(true).apply {
            setBackgroundColor(Pal.BG)
            addView(header)
            addView(ScrollView(act).apply { addView(body) }, ui.lp(Ui.MATCH, 0, 1f))
            if (footer != null) addView(footer)
        }
        dialog.setContentView(root)
        dialog.show()
    }
}

/**
 * Escolha do alvo: aviso de salvar na frente do alvo, alvos agrupados
 * (lendários, iniciais, presentes, selvagem) e, embaixo, "Como usar" + "Caçar X".
 */
class HuntTargetScreen(act: Activity, host: Host) {
    private val impl = object : HunterDialog(act, host) {
        private val targets = host.huntTargetList()
        private var selected = 0
        private val goButton = ui.accentButton("") { start() }
        private val tiles = mutableListOf<android.widget.TextView>()

        init {
            dialog.setOnDismissListener { host.onHunterClosed(closingToMenu) }
        }

        fun open() {
            val body = ui.box(true, 16).apply {
                setPadding(ui.dp(16), ui.dp(8), ui.dp(16), ui.dp(16))
                addView(ui.box(false, 10).apply {
                    background = ui.rounded(0x1A78C8FF, 12, 0x5978C8FF)
                    setPadding(ui.dp(14), ui.dp(12), ui.dp(14), ui.dp(12))
                    addView(ui.icon(R.drawable.ic_info, Pal.ACCENT, 20))
                    addView(ui.text(t(R.string.hunt_save_first), 13f, 0xFFD6E9F7.toInt()), ui.lp(0, Ui.WRAP, 1f))
                })
                val groups = listOf(
                    HuntTarget.LEGENDARY to R.string.hunt_group_legendary,
                    HuntTarget.STARTER to R.string.hunt_group_starter,
                    HuntTarget.GIFT to R.string.hunt_group_gift,
                    HuntTarget.WILD to R.string.hunt_group_wild,
                )
                for ((kind, titleRes) in groups) {
                    val idx = targets.indices.filter { targets[it].kind == kind }
                    if (idx.isEmpty()) continue
                    addView(ui.box(true, 6).apply {
                        addView(ui.sectionLabel(t(titleRes)))
                        addView(grid(idx))
                    })
                }
            }
            val footer = ui.box(false, 10).apply {
                setBackgroundColor(Pal.BG)
                setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(20))
                addView(FrameLayout(act).apply {
                    background = ui.touchable(ui.rounded(0x00000000, 14, Pal.BORDER))
                    contentDescription = t(R.string.hunt_how_to)
                    addView(ui.text("?", 20f, bold = true).apply {
                        layoutParams = FrameLayout.LayoutParams(Ui.WRAP, Ui.WRAP, Gravity.CENTER)
                    })
                    setOnClickListener { host.huntHelp(selected) { start() } }
                }, LinearLayout.LayoutParams(ui.dp(56), ui.dp(52)))
                addView(goButton, ui.lp(0, ui.dp(52), 1f))
            }
            paint()
            show(t(R.string.hunter_title), t(R.string.hunt_pick_sub, host.huntGame), body, footer)
        }

        private fun grid(indices: List<Int>) = GridLayout(act).apply {
            columnCount = 2
            val gap = ui.dp(3)
            for (i in indices) {
                val tile = ui.text(targets[i].name, 14f, bold = true).apply {
                    gravity = Gravity.CENTER_VERTICAL
                    minHeight = ui.dp(44)
                    setPadding(ui.dp(14), 0, ui.dp(14), 0)
                    tag = i
                    setOnClickListener { selected = i; paint() }
                }
                tiles += tile
                addView(tile, GridLayout.LayoutParams(
                    GridLayout.spec(GridLayout.UNDEFINED), GridLayout.spec(GridLayout.UNDEFINED, 1f),
                ).apply {
                    width = 0
                    setMargins(gap, gap, gap, gap)
                })
            }
        }

        private fun paint() {
            tiles.forEach { tile ->
                val on = tile.tag == selected
                tile.background = ui.touchable(ui.rounded(if (on) Pal.ACCENT_SOFT else Pal.SURFACE, 12, if (on) Pal.ACCENT else Pal.SURFACE, 2))
                tile.setTextColor(if (on) Pal.ACCENT else Pal.TEXT)
            }
            goButton.text = t(R.string.hunt_go, targets.getOrNull(selected)?.name ?: "")
        }

        private fun start() {
            host.startHunt(selected)
            closingToMenu = false
            dialog.dismiss()
        }
    }

    fun show() = impl.open()
}

/**
 * A caça parou antes de começar: com o time cheio, o presente iria pro PC, que
 * a caça não enxerga. Explica e oferece voltar a jogar ou escolher outro alvo.
 */
class PartyFullScreen(act: Activity, host: Host, private val target: String) {
    private val impl = object : HunterDialog(act, host) {
        private var pickOther = false

        init {
            dialog.setOnDismissListener { host.onPartyFullClosed(pickOther) }
        }

        override fun back() = dialog.dismiss()

        fun open() {
            val warn = 0xFFF0B46A.toInt()
            val body = ui.box(true, 16).apply {
                setPadding(ui.dp(16), ui.dp(8), ui.dp(16), ui.dp(16))
                addView(ui.card(10, 16).apply {
                    background = ui.rounded(Pal.SURFACE, 18, warn, 2)
                    addView(ui.box(false, 12).apply {
                        gravity = Gravity.CENTER_VERTICAL
                        addView(ui.icon(R.drawable.ic_warning, warn, 28))
                        addView(ui.text(t(R.string.party_full_title), 20f, bold = true))
                    })
                    addView(ui.text(t(R.string.party_full_why, target), 14f, 0xFFC8C8D0.toInt()))
                })
                addView(ui.card(14).apply {
                    addView(ui.sectionLabel(t(R.string.party_fix_title)))
                    listOf(R.string.party_step1, R.string.party_step2, R.string.party_step3).forEachIndexed { i, res ->
                        addView(ui.box(false, 12).apply {
                            addView(ui.text("${i + 1}", 13f, Pal.ACCENT, bold = true).apply {
                                gravity = Gravity.CENTER
                                background = ui.rounded(Pal.SURFACE2, 13)
                            }, LinearLayout.LayoutParams(ui.dp(26), ui.dp(26)))
                            addView(ui.text(t(res), 14f), ui.lp(0, Ui.WRAP, 1f))
                        })
                    }
                })
            }
            val footer = ui.box(true, 10).apply {
                setPadding(ui.dp(16), ui.dp(12), ui.dp(16), ui.dp(20))
                addView(ui.accentButton(t(R.string.party_back)) { dialog.dismiss() }, ui.lp(Ui.MATCH, ui.dp(52)))
                addView(ui.outlineButton(t(R.string.party_other)) {
                    pickOther = true
                    dialog.dismiss()
                }, ui.lp(Ui.MATCH, ui.dp(52)))
            }
            show(t(R.string.hunter_title), null, body, footer)
        }
    }

    fun show() = impl.open()
}
