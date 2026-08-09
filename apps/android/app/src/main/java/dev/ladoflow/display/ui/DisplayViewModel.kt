package dev.ladoflow.display.ui

import androidx.lifecycle.ViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

class DisplayViewModel : ViewModel() {
    private val mutableState = MutableStateFlow(DisplayUiState())
    val state: StateFlow<DisplayUiState> = mutableState.asStateFlow()

    fun accept(event: DisplayEvent) {
        mutableState.update { current -> DisplayStateMachine.reduce(current, event) }
    }
}
