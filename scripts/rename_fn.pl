#!/usr/bin/env perl
use strict;
use warnings;
use Getopt::Long;
use File::Find;
use File::Basename;
my $dirname = dirname(__FILE__);

my ($opt_old, $opt_new, $opt_file, $opt_comments);
GetOptions(
    "old=s" => \$opt_old,
    "new=s" => \$opt_new,
    "file=s" => \$opt_file,
    "comments" => \$opt_comments,
) or die "Usage: $0 --old <old_name> --new <new_name> [--file <file_path>] [--comments]\n";

die "Missing --old or --new\n" unless $opt_old && $opt_new;

# Find files with function definition
my $def_pattern = "fn\\s+\\Q$opt_old\\E\\s*\\(";
my $rg_cmd = "rg --type rust -l '$def_pattern'";
my $output = qx($rg_cmd);
my @def_files = grep { $_ } split /\n/, $output;

if (@def_files > 1 && !defined $opt_file) {
    die "Ambiguous function '$opt_old' found in multiple files: " . join(", ", @def_files) . "\nPlease specify --file <path>\n";
}

if (defined $opt_file) {
    my $found = 0;
    foreach my $f (@def_files) {
        if ($f eq $opt_file) {
            $found = 1;
            last;
        }
    }
    die "Function '$opt_old' not defined in '$opt_file'\n" unless $found;
}

# Find all .rs files
my @rs_files;
find(sub {
    push @rs_files, $File::Find::name if /\.rs$/ && -f;
}, '.');

# For each .rs file, perform the rename
foreach my $file (@rs_files) {
    open my $in, '<', $file or die "Can't open $file: $!\n";
    local $/;
    my $content = <$in>;
    close $in;

    my $len = length $content;
    my $pos = 0;
    my @spans = ();
    my $type = 'code';
    my $start = 0;
    my $depth = 0;

    my $hash_count = 0;  # for raw strings
    while ($pos < $len) {
        my $char = substr($content, $pos, 1);
        my $next = ($pos + 1 < $len) ? substr($content, $pos + 1, 1) : '';

        if ($type eq 'code') {
            if ($char eq '/' && $next eq '/') {
                if ($pos > $start) {
                    push @spans, {type => 'code', start => $start, end => $pos};
                }
                $type = 'line_comment';
                $start = $pos;
                $pos += 2;
                next;
            } elsif ($char eq '/' && $next eq '*') {
                if ($pos > $start) {
                    push @spans, {type => 'code', start => $start, end => $pos};
                }
                $type = 'block_comment';
                $depth = 1;
                $start = $pos;
                $pos += 2;
                next;
            } elsif ($char eq '"') {
                # Normal string
                if ($pos > $start) {
                    push @spans, {type => 'code', start => $start, end => $pos};
                }
                $type = 'string';
                $start = $pos;
                $pos++;
                next;
            } elsif ($char eq "'") {
                # Char literal
                if ($pos > $start) {
                    push @spans, {type => 'code', start => $start, end => $pos};
                }
                $type = 'char';
                $start = $pos;
                $pos++;
                next;
            } elsif ($char eq 'r' || $char eq 'b') {
                my $is_byte = ($char eq 'b');
                my $p = $pos + 1;
                if ($is_byte && substr($content, $p, 1) eq 'r') {
                    $p++;
                } elsif ($is_byte && substr($content, $p, 1) eq "'") {
                    # b'
                    if ($pos > $start) {
                        push @spans, {type => 'code', start => $start, end => $pos};
                    }
                    $type = 'byte_char';
                    $start = $pos;
                    $pos = $p + 1;
                    next;
                } elsif ($is_byte && substr($content, $p, 1) eq '"') {
                    # b"
                    if ($pos > $start) {
                        push @spans, {type => 'code', start => $start, end => $pos};
                    }
                    $type = 'byte_string';
                    $start = $pos;
                    $pos = $p + 1;
                    next;
                }
                $hash_count = 0;
                while (substr($content, $p, 1) eq '#') {
                    $hash_count++;
                    $p++;
                }
                if (substr($content, $p, 1) eq '"') {
                    # Raw or byte raw string
                    if ($pos > $start) {
                        push @spans, {type => 'code', start => $start, end => $pos};
                    }
                    $type = $is_byte ? 'byte_raw_string' : 'raw_string';
                    $start = $pos;
                    $pos = $p + 1;
                    next;
                } else {
                    $pos++;  # Not a string, continue
                    next;
                }
            } else {
                $pos++;
                next;
            }
        } elsif ($type eq 'line_comment') {
            $pos++;
            if ($char eq "\n") {
                push @spans, {type => 'comment', start => $start, end => $pos};
                $type = 'code';
                $start = $pos;
            }
            next;
        } elsif ($type eq 'block_comment') {
            if ($char eq '/' && $next eq '*') {
                $depth++;
                $pos += 2;
                next;
            } elsif ($char eq '*' && $next eq '/') {
                $depth--;
                $pos += 2;
                if ($depth == 0) {
                    push @spans, {type => 'comment', start => $start, end => $pos};
                    $type = 'code';
                    $start = $pos;
                }
                next;
            } else {
                $pos++;
                next;
            }
        } elsif ($type eq 'string' || $type eq 'byte_string') {
            if ($char eq '\\') {
                $pos += 2;
                next;
            } elsif ($char eq '"') {
                $pos++;
                push @spans, {type => 'string', start => $start, end => $pos};
                $type = 'code';
                $start = $pos;
                next;
            } else {
                $pos++;
                next;
            }
        } elsif ($type eq 'char' || $type eq 'byte_char') {
            if ($char eq '\\') {
                $pos += 2;
                next;
            } elsif ($char eq "'") {
                $pos++;
                push @spans, {type => 'char', start => $start, end => $pos};
                $type = 'code';
                $start = $pos;
                next;
            } else {
                $pos++;
                next;
            }
        } elsif ($type eq 'raw_string' || $type eq 'byte_raw_string') {
            $pos++;
            if ($char eq '"') {
                my $h = 0;
                my $p = $pos + 1;
                while ($h < $hash_count && substr($content, $p, 1) eq '#') {
                    $h++;
                    $p++;
                }
                if ($h == $hash_count) {
                    $pos = $p;
                    push @spans, {type => 'string', start => $start, end => $pos};
                    $type = 'code';
                    $start = $pos;
                    next;
                }
            }
            next;
        }
    }
    if ($start < $len) {
        my $final_type = ($type eq 'line_comment' || $type eq 'block_comment') ? 'comment' : $type;
        push @spans, {type => $final_type, start => $start, end => $len};
    }
    my $new_content = '';
    foreach my $span (@spans) {
        my $text = substr($content, $span->{start}, $span->{end} - $span->{start});
        if ($span->{type} eq 'code') {
            $text =~ s/(\b\Q$opt_old\E)(?=\s*\()/$opt_new/g;
        } elsif ($span->{type} eq 'comment' && $opt_comments) {
            $text =~ s/\b\Q$opt_old\E\b/$opt_new/g;
        }
        $new_content .= $text;
    }

    open my $out, '>', $file or die "Can't write to $file: $!\n";
    print $out $new_content;
    close $out;
}

my $commit_msg = "Rename function $opt_old to $opt_new";
$commit_msg .= " (including comments)" if $opt_comments;
if (defined $opt_file) {
    $commit_msg .= " in $opt_file";
}

# Run cargo build
system("$dirname/build_and_commit.sh \"$commit_msg\"");

