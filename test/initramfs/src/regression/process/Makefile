# SPDX-License-Identifier: MPL-2.0

SUBDIRS := \
	clone3 \
	cpu_affinity \
	execve \
	exit \
	fork_c \
	getcpu \
	get_robust_list \
	getpid \
	personality \
	prctl \
	pthread \
	ptrace \
	sched \
	signal \
	sigqueueinfo \

ifeq ($(HOST_PLATFORM), x86_64-linux)
SUBDIRS += \
	arch_prctl \
	fork
endif

include ../common/Makefile
