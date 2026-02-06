#!/usr/bin/env perl

# Summary - when the Perl script is likely to break
# Context	                 Blind text replace safe?	Typical failure mode
# Normal function calls	     Yes	                    -
# Method calls (self.old())	 Yes (with lookahead)	    -
# macro_rules! body	         Usually	                Dynamic construction (paste!, etc.)
# Proc-macro implementation	 No	                        Name in variables, Ident, quote! tokens
# Proc-macro generated code	 Sometimes	                Name constructed at expand time
# Macro invocation argument	 Sometimes	                Name computed inside macro
# use crate::...::old_name;	 Yes	                    - (if followed by ( in lookahead version)
# Strings / comments (opt)	 Controlled by flag	        Unintended renames if over-applied

use strict;
use warnings;
use Getopt::Long;
use File::Find;
use File::Basename;
my $dirname = dirname(__FILE__);

my ($opt_old, $opt_new, $opt_file, $opt_comments, $source_dir);
$source_dir = 'src/';  # default to current directory
GetOptions(
    "old=s" => \$opt_old,
    "new=s" => \$opt_new,
    "file=s" => \$opt_file,
    "comments" => \$opt_comments,
    "source-dir=s" => \$source_dir,
) or die "Usage: $0 --old <old_name> --new <new_name> [--file <file_path>] [--comments]\n";

die "Missing --old or --new\n" unless $opt_old && $opt_new;

if ($opt_old !~ /^[A-Za-z_][A-Za-z0-9_]*$/) {
    die "Old function name '$opt_old' contains special characters. Aborting.\n";
}
if ($opt_new !~ /^[A-Za-z_][A-Za-z0-9_]*$/) {
    die "New function name '$opt_new' contains special characters. Aborting.\n";
}
if (defined $source_dir && !-d $source_dir) {
    die "Source directory '$source_dir' does not exist.\n";
}

# Find files with function definition
my $def_pattern = '^\s*(?:pub(?:\s*\([^)]+\))?\s+)?(?:async\s+)?fn\s+'.$opt_old.'\s*\(';
my $rg_cmd = 'rg --type rust -l "'.$def_pattern.'"';
my $output = qx($rg_cmd);
my @def_files = grep { $_ } split /\n/, $output;
print STDERR "Found function '$opt_old' in files: " . join(", ", @def_files) . "\n";
my @files_to_process;

if (@def_files > 1 and not defined $opt_file) {
    die "Ambiguous function '$opt_old' found in multiple files: " . join(", ", @def_files) . "\nPlease specify --file <path>\n";
}

if (defined $opt_file) {
    die "Specified file '$opt_file' does not exist.\n" unless -f $opt_file;
    my $cmd = "rg --type rust -l '$def_pattern' -- $opt_file";
    my $res = qx{$cmd};
    die "Definition of '$opt_old' not found in $opt_file\n" unless $res =~ /\S/;
    @def_files = ($opt_file);
}

my $found_def = 0;
foreach my $f (@def_files) {
    my $cmd = qq{rg --type rust -l -F -- 'fn $opt_old' '$f'};
    my $res = qx{$cmd};
    if ($res =~ /\S/) {
        $found_def = 1;
        last;
    }
}
die "No definition of '$opt_old' found in target file(s). Aborting.\n" unless $found_def;

my @rs_files;
find(sub { push @rs_files, $File::Find::name if /\.rs$/ && -f;}, 'src/');

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
            # If we have already consumed content ($pos > $start)
            # and the current char is NOT a closing quote, this is a lifetime.
            if ($pos > $start && $char ne "'") {
                # Close the previous segment as a 'lifetime' (or distinct code span)
                push @spans, {type => 'lifetime', start => $start, end => $pos};

                # Switch state back to code
                $type = 'code';
                $start = $pos;

                # Restart the loop for this SAME character so it gets parsed as code
                redo;
            }

            if ($char eq '\\') {
                $pos += 2; # Skip escape (e.g., \n or \')
                next;
            } elsif ($char eq "'") {
                $pos++;
                push @spans, {type => $type, start => $start, end => $pos};
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
            if (defined $opt_file) {
                # Method rename mode: only replace method-style calls + the definition itself
                # 1. The definition (fn old_name( )
                $text =~ s{
                        \b                                      # word boundary before fn
                        ( pub (?: \s* \( [^)]* \) )? \s+ )?     # optional pub / pub(crate) / pub(super)
                        ( async \s+ )?                          # optional async
                        fn \s+
                        \Q$opt_old\E \b                         # the function name we're renaming
                        \s*                                     # optional whitespace
                        (?: < [^>]* > \s* )?                    # optional <'a, 'b, T> generics/lifetimes (non-greedy-ish)
                        \(                                      # start of parameter list - this is our anchor
                    }{$1$2fn $opt_new(}gx if $file eq $opt_file;
                # 2. Method calls: .old_name(   or ::old_name(
                $text =~ s{
                    (\s* [.:] \s*)          # . or :: possibly with ws
                    \b \Q$opt_old\E \b
                    (?=\s*\()               # followed by (
                }{$1$opt_new}gx;
                print STDERR "Renamed method '$opt_old' to '$opt_new' in $file\n" if $text =~ /\b$opt_new\b/;

                # Optional: also catch Self::old_name or Type::old_name more precisely
                # (but the above usually catches most)
            } else {
                # Free function mode: replace bare old_name( calls + definition
                # 1. Definition
                $text =~ s{
                        \b                                      # word boundary before fn
                        ( pub (?: \s* \( [^)]* \) )? \s+ )?     # optional pub / pub(crate) / pub(super)
                        ( async \s+ )?                          # optional async
                        fn \s+
                        \Q$opt_old\E \b                         # the function name we're renaming
                        \s*                                     # optional whitespace
                        (?: < [^>]* > \s* )?                    # optional <'a, 'b, T> generics/lifetimes (non-greedy-ish)
                        \(                                      # start of parameter list - this is our anchor
                    }{$1$2fn $opt_new(}gx;
                # 2. Bare calls (not after . or ::)
                $text =~ s{
                    (?<![.:]\s*)          # NOT preceded by . or :: (with optional ws)
                    \b\Q$opt_old\E\b
                    (?=\s*\()
                }{$opt_new}gx;
            }
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

