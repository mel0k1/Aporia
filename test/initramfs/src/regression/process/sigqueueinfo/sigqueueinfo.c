#define _GNU_SOURCE

#include <signal.h>
#include <string.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "../../common/test.h"

static volatile sig_atomic_t delivered_value;
static volatile sig_atomic_t delivered_code;

static void sigqueue_handler(int signo, siginfo_t *info, void *ucontext)
{
	(void)signo;
	(void)ucontext;
	delivered_value = info->si_value.sival_int;
	delivered_code = info->si_code;
}

FN_TEST(rt_sigqueueinfo_value_delivery)
{
	struct sigaction action = { 0 };
	struct sigaction old_action = { 0 };
	action.sa_sigaction = sigqueue_handler;
	action.sa_flags = SA_SIGINFO;
	sigemptyset(&action.sa_mask);
	TEST_SUCC(sigaction(SIGUSR1, &action, &old_action));

	siginfo_t info = { 0 };
	info.si_signo = SIGUSR1;
	info.si_code = SI_USER;
	info.si_value.sival_int = 42;
	TEST_RES(syscall(SYS_rt_sigqueueinfo, getpid(), SIGUSR1, &info),
		 _ret == 0);

	TEST_SUCC(usleep(100000));
	CHECK(delivered_value == 42);
	CHECK(delivered_code == SI_QUEUE);

	info.si_signo = SIGUSR2;
	TEST_ERRNO(syscall(SYS_rt_sigqueueinfo, getpid(), SIGUSR1, &info),
		   EINVAL);

	info.si_signo = SIGUSR1;
	info.si_code = SI_QUEUE;
	TEST_ERRNO(syscall(SYS_rt_sigqueueinfo, getpid(), SIGUSR1, &info),
		   EPERM);

	info.si_signo = 0;
	TEST_RES(syscall(SYS_rt_sigqueueinfo, getpid(), 0, &info), _ret == 0);

	TEST_SUCC(sigaction(SIGUSR1, &old_action, NULL));
}
END_TEST()

FN_TEST(rt_sigqueueinfo_err)
{
	siginfo_t info = { 0 };
	info.si_signo = SIGUSR1;
	info.si_code = SI_USER;

	TEST_ERRNO(syscall(SYS_rt_sigqueueinfo, -1, SIGUSR1, &info), EINVAL);
}
END_TEST()
