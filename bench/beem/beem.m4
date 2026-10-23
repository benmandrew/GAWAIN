divert(-1)
# The BEEM m4 macros, rewritten: the archive ships the models but not the
# macro file. default(N,v) defines N unless the instance sets it;
# forloop(i,a,b,body) and myloop(i,a,b,body,sep) repeat body for i = a..b,
# and myloop puts sep between the copies.
define(`default', `ifdef(`$1', , `define(`$1', `$2')')')
define(`forloop', `ifelse(eval(($2) <= ($3)), 1, `pushdef(`$1', `$2')_forloop(`$1', `$3', `$4')popdef(`$1')')')
define(`_forloop', `$3`'ifelse(eval($1 < ($2)), 1, `define(`$1', incr($1))_forloop(`$1', `$2', `$3')')')
define(`myloop', `ifelse(eval(($2) <= ($3)), 1, `pushdef(`$1', `$2')_myloop(`$1', `$3', `$4', `$5')popdef(`$1')')')
define(`_myloop', `$3`'ifelse(eval($1 < ($2)), 1, `$4`'define(`$1', incr($1))_myloop(`$1', `$2', `$3', `$4')')')
# buffer_channel(c,K) declares rendezvous channels c_in and c_out and a
# process channel_c that buffers up to K values in FIFO order. The process
# is diverted to the end of the output; where it lands does not matter.
define(`buffer_channel', `channel $1_in, $1_out;
divert(1)process channel_$1 {
byte buf[$2];
byte buf_act = 0;
state q;
init q;
trans
 q -> q { guard not(buf_act == $2); sync $1_in?buf[buf_act]; effect buf_act = buf_act+1; },
 q -> q { guard not(buf_act == 0); sync $1_out!buf[0]; effect myloop(`_bc_i', 0, eval($2-2), `buf[_bc_i] = buf[incr(_bc_i)]', `, '), buf[decr($2)] = 0, buf_act = buf_act-1;};
}
divert(0)')
divert(0)dnl
